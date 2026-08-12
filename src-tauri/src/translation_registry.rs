use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::sync::oneshot;

type CancelSender = Arc<Mutex<Option<oneshot::Sender<()>>>>;

#[derive(Default)]
pub struct TranslationRegistry {
    inflight: Mutex<HashMap<String, (u64, CancelSender)>>,
}

impl TranslationRegistry {
    // キャンセルは registry 経由(cancel/supersede)のみ。sender を外に返さないことで
    // 経路が1本であることを型で保証する。
    pub fn begin(&self, label: &str, request_id: u64, cancel_tx: oneshot::Sender<()>) {
        let cancel_tx: CancelSender = Arc::new(Mutex::new(Some(cancel_tx)));

        let mut inflight = self.inflight.lock().unwrap();
        if let Some((_, previous)) = inflight.remove(label) {
            if let Some(tx) = previous.lock().unwrap().take() {
                let _ = tx.send(());
            }
        }
        inflight.insert(label.to_string(), (request_id, cancel_tx));
    }

    pub fn finish(&self, label: &str, request_id: u64) {
        let mut inflight = self.inflight.lock().unwrap();
        if let Some((current_id, _)) = inflight.get(label) {
            if *current_id == request_id {
                inflight.remove(label);
            }
        }
    }

    // finish と同じ世代ガードを持つ。停止→即再翻訳の順でイベントが遅延到着しても、
    // 古い世代宛の cancel が新しいストリームを巻き込まないようにする。
    // request_id が None のときは無条件(ウィンドウ破棄時の掃除用)。
    pub fn cancel(&self, label: &str, request_id: Option<u64>) {
        let mut inflight = self.inflight.lock().unwrap();
        let should_cancel = match (request_id, inflight.get(label)) {
            (_, None) => false,
            (None, Some(_)) => true,
            (Some(id), Some((current_id, _))) => *current_id == id,
        };
        if !should_cancel {
            return;
        }
        if let Some((_, cancel_tx)) = inflight.remove(label) {
            if let Some(tx) = cancel_tx.lock().unwrap().take() {
                let _ = tx.send(());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use tokio::sync::mpsc;

    #[tokio::test]
    async fn superseding_a_stream_cancels_its_sender() {
        let registry = TranslationRegistry::default();
        let (tx1, mut rx1) = oneshot::channel();
        let (tx2, _rx2) = oneshot::channel();

        registry.begin("main", 1, tx1);
        registry.begin("main", 2, tx2);

        assert!(
            rx1.try_recv().is_ok(),
            "first generation must receive a cancel signal once superseded"
        );
    }

    #[tokio::test]
    async fn finish_ignores_stale_generation() {
        let registry = TranslationRegistry::default();
        let (tx1, _rx1) = oneshot::channel();
        let (tx2, mut rx2) = oneshot::channel();

        registry.begin("main", 1, tx1);
        registry.begin("main", 2, tx2);
        registry.finish("main", 1);

        registry.cancel("main", None);
        assert!(
            rx2.try_recv().is_ok(),
            "generation 2 must still be tracked and cancellable after a stale finish() from generation 1"
        );
    }

    #[tokio::test]
    async fn stale_cancel_does_not_kill_the_new_generation() {
        let registry = TranslationRegistry::default();
        let (tx2, mut rx2) = oneshot::channel();

        registry.begin("main", 2, tx2);
        registry.cancel("main", Some(1));

        assert!(
            rx2.try_recv().is_err(),
            "a cancel targeting generation 1 must not cancel generation 2"
        );

        registry.cancel("main", Some(2));
        assert!(
            rx2.try_recv().is_ok(),
            "a cancel targeting the current generation must go through"
        );
    }

    async fn run_stream(
        registry: Arc<TranslationRegistry>,
        label: &'static str,
        request_id: u64,
        words: Vec<&'static str>,
        events_tx: mpsc::UnboundedSender<(u64, &'static str)>,
        started_tx: Option<oneshot::Sender<()>>,
    ) {
        let (cancel_tx, mut cancel_rx) = oneshot::channel();
        registry.begin(label, request_id, cancel_tx);
        if let Some(started_tx) = started_tx {
            let _ = started_tx.send(());
        }

        for word in words {
            tokio::select! {
                biased;
                _ = &mut cancel_rx => break,
                _ = tokio::time::sleep(Duration::from_millis(15)) => {
                    let _ = events_tx.send((request_id, word));
                }
            }
        }
        registry.finish(label, request_id);
    }

    #[tokio::test]
    async fn superseded_stream_never_interleaves_chunks_with_the_new_one() {
        let registry = Arc::new(TranslationRegistry::default());
        let (events_tx, mut events_rx) = mpsc::unbounded_channel::<(u64, &'static str)>();
        let (started_tx, started_rx) = oneshot::channel();

        let r1 = registry.clone();
        let tx1 = events_tx.clone();
        let handle1 = tokio::spawn(run_stream(
            r1,
            "main",
            1,
            vec!["A1", "A2", "A3", "A4", "A5"],
            tx1,
            Some(started_tx),
        ));

        started_rx.await.unwrap();

        let r2 = registry.clone();
        let tx2 = events_tx.clone();
        run_stream(r2, "main", 2, vec!["B1", "B2", "B3"], tx2, None).await;

        let _ = handle1.await;
        drop(events_tx);

        let mut seen = Vec::new();
        while let Ok(item) = events_rx.try_recv() {
            seen.push(item);
        }

        assert!(
            seen.iter().all(|(id, _)| *id == 2),
            "generation 1 must not emit any chunk after being superseded, got {seen:?}"
        );
        assert_eq!(
            seen.iter().map(|(_, w)| *w).collect::<Vec<_>>(),
            vec!["B1", "B2", "B3"]
        );
    }
}
