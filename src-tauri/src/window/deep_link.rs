use tauri::Url;

const SCHEME: &str = "trapop";
const NEW_PANEL_HOST: &str = "new";

// `trapop://new` だけをパネル起動要求として扱う。他のホスト/パスは無視する
pub fn is_new_panel_url(url: &Url) -> bool {
    url.scheme() == SCHEME && url.host_str() == Some(NEW_PANEL_HOST)
}
