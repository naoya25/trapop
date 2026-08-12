// コードエディタ/ターミナルからの貼り付けは、平文を div/span で包んだだけの
// text/html を載せてくる。これを HTML モードで翻訳すると、モデルに 35k 超の
// span スープが渡って echo される・markdown が描画されない、の両方が起きる。
// 見出し/リスト/表など意味のあるタグが1つも無い HTML は「リッチではない」と
// 判定し、呼び出し側で平文モードに落とす。
const SEMANTIC_TAG =
  /<(h[1-6]|ul|ol|li|table|thead|tbody|tr|td|th|a|strong|b|em|i|code|pre|blockquote|img|hr)\b/i;

export function htmlLooksRich(html: string): boolean {
  return SEMANTIC_TAG.test(html);
}

// タグ剥がしは HTML モードのみ。plain/markdown の `a < b` 等のコード片を
// 巻き込まない(backend history.rs の is_html 分岐と同じ判断)。
export function safeStreamPreview(buffer: string, isHtml: boolean): string {
  if (!isHtml) {
    return buffer;
  }
  const lastOpen = buffer.lastIndexOf("<");
  const lastClose = buffer.lastIndexOf(">");
  const safeBuffer = lastOpen > lastClose ? buffer.slice(0, lastOpen) : buffer;
  return safeBuffer.replace(/<[^>]*>/g, "");
}
