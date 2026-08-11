import test from "node:test";
import assert from "node:assert/strict";
import { safeStreamPreview } from "./preview.ts";

test("passes plain text through unchanged", () => {
  assert.equal(safeStreamPreview("hello world"), "hello world");
});

test("strips complete tags while keeping their text content", () => {
  assert.equal(safeStreamPreview("<div>ケーキを</div><div>食べました</div>"), "ケーキを食べました");
});

test("drops a tag left dangling open at the end of the buffer", () => {
  assert.equal(safeStreamPreview("こんにちは<di"), "こんにちは");
});

test("drops a dangling closing tag with no '>' yet", () => {
  assert.equal(safeStreamPreview("text</div"), "text");
});

test("never surfaces a broken tag fragment across chunk boundaries", () => {
  const chunks = ["私は今日の昼食にハーブ", "<div>ケーキを", "</div><div>", "食べました。</div>"];
  let buffer = "";
  const previews: string[] = [];

  for (const chunk of chunks) {
    buffer += chunk;
    previews.push(safeStreamPreview(buffer));
  }

  for (const preview of previews) {
    assert.ok(!preview.includes("<"), `preview must not contain '<': ${preview}`);
    assert.ok(!preview.includes(">"), `preview must not contain '>': ${preview}`);
  }

  assert.equal(previews.at(-1), "私は今日の昼食にハーブケーキを食べました。");
});
