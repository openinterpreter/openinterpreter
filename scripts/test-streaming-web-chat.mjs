#!/usr/bin/env node
// Exercise the exact two fenced files readers copy from the guide, not a fork.
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import {
  chmodSync,
  mkdtempSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import vm from "node:vm";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const guide = readFileSync(join(root, "docs/streaming-web-chat.md"), "utf8");
function fence(name, language) {
  const match = guide.match(
    new RegExp(
      "### `" + name + "`\\s*```" + language + "\\n([\\s\\S]*?)\\n```",
    ),
  );
  assert.ok(match, `missing ${name} fence`);
  return match[1];
}
const bridge = fence("bridge.mjs", "js"),
  html = fence("index.html", "html");
const script = html.match(/<script>([\s\S]*?)<\/script>/)?.[1];
assert.ok(script, "missing browser script");
new vm.Script(script); // syntax-check the exact browser sample

class Element {
  constructor() {
    this.children = [];
    this.parent = null;
    this.textContent = "";
    this._top = 0;
    this.clientHeight = 100;
  }
  get scrollTop() {
    return this._top;
  }
  set scrollTop(value) {
    this._top = Math.max(
      0,
      Math.min(value, this.scrollHeight - this.clientHeight),
    );
  }
  get offsetHeight() {
    return Math.max(
      40,
      Math.ceil(
        (this.textContent.length +
          this.children.reduce((n, x) => n + x.textContent.length, 0)) /
          80,
      ) * 20,
    );
  }
  get scrollHeight() {
    return this.children.reduce((n, x) => n + x.offsetHeight + 16, 0);
  }
  get firstElementChild() {
    return this.children[0];
  }
  getBoundingClientRect() {
    return {
      top:
        this.parent.children
          .slice(0, this.parent.children.indexOf(this))
          .reduce((n, x) => n + x.offsetHeight + 16, 0) - this.parent.scrollTop,
    };
  }
  append(...items) {
    for (const item of items) {
      item.parent = this;
      this.children.push(item);
    }
  }
  remove() {
    const p = this.parent;
    p.children.splice(p.children.indexOf(this), 1);
    p.scrollTop = p.scrollTop;
    this.parent = null;
  }
  setAttribute() {}
}
const box = new Element(),
  history = new Element(),
  input = { value: "" },
  button = { disabled: false };
const form = {
  querySelector: (sel) => (sel === "textarea" ? input : button),
  addEventListener: (_, handler) => {
    form.submit = handler;
  },
};
const frames = new Map();
let frameId = 0,
  requested = 0,
  events = [];
const context = {
  document: {
    querySelector: (sel) =>
      sel === "form" ? form : sel === "#history" ? history : box,
    createElement: () => new Element(),
  },
  TextDecoder,
  requestAnimationFrame: (fn) => {
    frames.set(++frameId, fn);
    requested++;
    return frameId;
  },
  cancelAnimationFrame: (id) => frames.delete(id),
  fetch: async () => ({
    ok: true,
    body: {
      getReader() {
        const bytes = new TextEncoder().encode(
          events.map((e) => JSON.stringify(e) + "\n").join(""),
        );
        let i = 0;
        return {
          read: async () =>
            i < bytes.length
              ? { value: bytes.slice(i, ++i), done: false }
              : { done: true },
          cancel: async () => {},
        };
      },
    },
  }),
};
vm.runInNewContext(script, context);
async function submit(prompt, list) {
  input.value = prompt;
  events = list;
  await form.submit({ preventDefault() {} });
  for (const paint of frames.values()) paint();
  frames.clear();
  assert.equal(button.disabled, false);
}
for (let i = 0; i < 1000; i++) {
  await submit(`prompt ${i}`, [
    { type: "text", text: `answer ${i} 😀` },
    { type: "done", status: "completed" },
  ]);
  assert.ok(box.children.length <= 8);
}
assert.match(history.textContent, /Earlier visible turns trimmed/);
assert.match(box.children.at(-1).children[1].textContent, /answer 999 😀/);
await submit("long", [
  { type: "text", text: "ñ".repeat(30000) },
  { type: "done", status: "completed" },
]);
assert.ok(box.children.at(-1).children[1].textContent.length <= 12100);
assert.match(
  box.children.at(-1).children[1].textContent,
  /Earlier answer text trimmed/,
);
assert.doesNotMatch(box.children.at(-1).children[1].textContent, /\ufffd/);
// Move the long answer to the oldest slot. Removing it clamps scrollTop well
// before the browser code can restore the surviving row's visible position.
for (let i = 0; i < 7; i++)
  await submit(`filler ${i}`, [{ type: "done", status: "completed" }]);
box.scrollTop = box.scrollHeight - box.clientHeight - 60;
const anchor = box.children.at(-2),
  top = anchor.getBoundingClientRect().top;
await submit("reader", [
  { type: "text", text: "z".repeat(300) },
  { type: "done", status: "completed" },
]);
assert.equal(anchor.getBoundingClientRect().top, top);
await submit("EOF", [{ type: "text", text: "partial" }]);
assert.match(
  box.children.at(-1).children[2].textContent,
  /Stream closed without a complete final event/,
);
await submit("approval", [
  { type: "error", text: "Declined: approval" },
  { type: "done", status: "completed" },
]);
assert.match(
  box.children.at(-1).children[2].textContent,
  /Complete \(notice: Declined: approval\)/,
);
await submit("approval EOF", [{ type: "error", text: "Declined: approval" }]);
assert.match(
  box.children.at(-1).children[2].textContent,
  /Stream closed without a complete final event/,
);
await submit("limit", [
  { type: "error", fatal: true, text: "64 KiB cap" },
  { type: "done", status: "completed" },
]);
assert.match(
  box.children.at(-1).children[2].textContent,
  /Interrupted.*64 KiB/,
);
assert.ok(requested < 1200, "paints should batch rather than paint per byte");

const dir = mkdtempSync(join(tmpdir(), "oix-doc-chat-"));
let child;
try {
  writeFileSync(join(dir, "bridge.mjs"), bridge);
  writeFileSync(join(dir, "index.html"), html);
  const fake = join(dir, "fake.mjs");
  writeFileSync(
    fake,
    `#!/usr/bin/env node
import { createInterface } from 'node:readline';
let turn = 0;
const send = m => process.stdout.write(JSON.stringify(m) + '\\n');
for await (const line of createInterface({ input: process.stdin })) {
  const m = JSON.parse(line);
  if (m.method === 'initialize') send({ id: m.id, result: {} });
  if (m.method === 'thread/start') send({ id: m.id, result: { thread: { id: 't' } } });
  if (m.method === 'turn/start') {
    const id = 'turn-' + ++turn, text = m.params.input[0].text;
    if (text === 'early') send({ method: 'item/agentMessage/delta', params: { threadId: 't', turnId: id, delta: 'y'.repeat(17000) } });
    send({ id: m.id, result: { turn: { id } } });
    if (text === 'early') continue;
    // Let the bridge process turn/start before its normal stream. Only the
    // explicit 'early' case should exercise the pre-response queue limit.
    await new Promise(resolve => setTimeout(resolve, 20));
    for (let i = 0, n = text === 'large' ? 71000 : 30; i < n; i += 1000)
      send({ method: 'item/agentMessage/delta', params: { threadId: 't', turnId: id, delta: 'x'.repeat(Math.min(1000, n - i)) } });
    if (text !== 'large') send({ method: 'turn/completed', params: { threadId: 't', turn: { id, status: 'completed' } } });
  }
  if (m.method === 'turn/interrupt') {
    send({ id: m.id, result: {} });
    send({ method: 'turn/completed', params: { threadId: 't', turn: { id: m.params.turnId, status: 'interrupted' } } });
  }
}`,
  );
  chmodSync(fake, 0o755);
  child = spawn(process.execPath, [join(dir, "bridge.mjs")], {
    env: { ...process.env, OIX_BINARY: fake },
    stdio: ["ignore", "pipe", "pipe"],
  });
  const ready = new Promise((yes, no) => {
    let output = "";
    child.stdout.on("data", (data) => {
      output += data;
      if (output.includes("http://127.0.0.1:8787")) yes();
    });
    child.on("exit", (code) => no(Error(`bridge exited ${code}`)));
    setTimeout(
      () => no(Error(`bridge startup timeout: ${output}`)),
      5000,
    ).unref();
  });
  await ready;
  async function turn(prompt) {
    const result = await fetch("http://127.0.0.1:8787/chat", {
      method: "POST",
      headers: {
        "Host": "127.0.0.1:8787",
        "Origin": "http://127.0.0.1:8787",
        "Content-Type": "text/plain",
      },
      body: prompt,
    });
    assert.equal(result.status, 200);
    return (await result.text()).trim().split("\n").map(JSON.parse);
  }
  assert.equal((await turn("short")).at(-1).type, "done");
  assert.match((await turn("large")).at(-1).text, /64 KiB/);
  assert.match((await turn("early")).at(-1).text, /Too many early events/);
  assert.equal((await turn("short")).at(-1).type, "done");
} finally {
  child?.kill();
  rmSync(dir, { recursive: true, force: true });
}
console.log(
  "PASS: exact guide fences; bounded 1000-turn display, UTF-8, scroll anchoring, EOF, fatal limits, bridge transport",
);
