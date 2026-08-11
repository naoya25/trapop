export function safeStreamPreview(buffer: string): string {
  const lastOpen = buffer.lastIndexOf("<");
  const lastClose = buffer.lastIndexOf(">");
  const safeBuffer = lastOpen > lastClose ? buffer.slice(0, lastOpen) : buffer;
  return safeBuffer.replace(/<[^>]*>/g, "");
}
