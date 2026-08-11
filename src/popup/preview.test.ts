import test from "node:test";
import assert from "node:assert/strict";
import { safeStreamPreview } from "./preview.ts";

test("passes plain text through unchanged", () => {
  assert.equal(safeStreamPreview("hello world", false), "hello world");
});

test("keeps angle brackets in plain mode (code snippets survive)", () => {
  assert.equal(
    safeStreamPreview("if (a < b) return c > d;", false),
    "if (a < b) return c > d;",
  );
});

test("strips complete tags while keeping their text content", () => {
  assert.equal(
    safeStreamPreview("<div>ケーキを</div><div>食べました</div>", true),
    "ケーキを食べました",
  );
});

test("drops a tag left dangling open at the end of the buffer", () => {
  assert.equal(safeStreamPreview("こんにちは<di", true), "こんにちは");
});

test("drops a dangling closing tag with no '>' yet", () => {
  assert.equal(safeStreamPreview("text</div", true), "text");
});

test("never surfaces a broken tag fragment across chunk boundaries", () => {
  const chunks = ["私は今日の昼食にハーブ", "<div>ケーキを", "</div><div>", "食べました。</div>"];
  let buffer = "";
  const previews: string[] = [];

  for (const chunk of chunks) {
    buffer += chunk;
    previews.push(safeStreamPreview(buffer, true));
  }

  for (const preview of previews) {
    assert.ok(!preview.includes("<"), `preview must not contain '<': ${preview}`);
    assert.ok(!preview.includes(">"), `preview must not contain '>': ${preview}`);
  }

  assert.equal(previews.at(-1), "私は今日の昼食にハーブケーキを食べました。");
});
