import test from "node:test";
import assert from "node:assert/strict";
import { htmlLooksRich, safeStreamPreview } from "./preview.ts";

test("editor-style span soup is not rich html", () => {
  const soup =
    '<div><div><span># 見出しテキスト</span></div><br><div><span>**強調**</span><span>: 本文が続く</span></div></div>';
  assert.equal(htmlLooksRich(soup), false);
});

test("browser-style rich html is detected", () => {
  assert.equal(htmlLooksRich("<h1>Title</h1><p>body</p>"), true);
  assert.equal(htmlLooksRich("<div><ul><li>item</li></ul></div>"), true);
  assert.equal(htmlLooksRich('<p><a href="https://example.com">link</a></p>'), true);
  assert.equal(htmlLooksRich("<div><strong>bold</strong></div>"), true);
});

test("tag-name prefixes do not false-positive rich detection", () => {
  // <br> は <b の前方一致、<i…> 系は <i の前方一致になりやすい
  assert.equal(htmlLooksRich("<div>a<br>b</div>"), false);
  assert.equal(htmlLooksRich("<div><input value='x'></div>"), false);
});

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
