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
