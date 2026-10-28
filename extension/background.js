// Card English for Chrome, Edge, Brave and Coc Coc: a menu item on selected text that sends the word, the sentence
// it is in and the address of the page to the app, through native messaging to the app itself (src/browser.rs).
"use strict";

const HOST = "com.eliasbui.cardenglish";
const MENU = "card-english-add";

chrome.runtime.onInstalled.addListener(() => {
  chrome.contextMenus.create({ id: MENU, title: "Thêm vào Card English", contexts: ["selection"] });
});

// Runs in the page: the sentence around the selection, from the text of the block it is in. A selection inside a
// text box has no range in the page, so it gets none.
function sentenceOfSelection() {
  const selection = getSelection();
  if (!selection || !selection.rangeCount) return "";
  const range = selection.getRangeAt(0);
  const picked = range.toString();
  if (!picked.trim()) return "";
  const node = range.startContainer;
  const element = node.nodeType === Node.TEXT_NODE ? node.parentElement : node;
  const block = element?.closest("p, li, td, th, dd, dt, blockquote, figcaption, h1, h2, h3, h4, h5, h6, pre, div, article, section") || document.body;
  const before = document.createRange();
  before.setStart(block, 0);
  before.setEnd(range.startContainer, range.startOffset);
  const text = block.textContent || "";
  const start = before.toString().length, end = start + picked.length;
  // Out to the marks that end a sentence, but never further than a few lines from the word.
  const stop = /[.!?。！？…]/;
  let from = start, to = end;
  while (from > Math.max(0, start - 250) && !stop.test(text[from - 1])) from--;
  while (to < Math.min(text.length, end + 250) && !stop.test(text[to])) to++;
  if (to < text.length && stop.test(text[to])) to++;
  return text.slice(from, to).replace(/\s+/g, " ").trim();
}

// Runs in the page: a short note in a corner of it, saying what happened.
function noteInPage(message, ok) {
  const box = document.createElement("div");
  box.textContent = message;
  box.setAttribute("role", "status");
  box.style.cssText = "all:initial;position:fixed;z-index:2147483647;top:16px;right:16px;max-width:360px;padding:11px 16px;border-radius:12px;"
    + "font:600 14px/1.45 system-ui,-apple-system,'Segoe UI',sans-serif;box-shadow:0 10px 28px rgba(0,0,0,.28);"
    + (ok ? "background:#141820;color:#fff;" : "background:#c42b2b;color:#fff;");
  document.documentElement.append(box);
  setTimeout(() => box.remove(), 5000);
}

function send(message) {
  return new Promise((resolve) => {
    chrome.runtime.sendNativeMessage(HOST, message, (reply) => {
      if (chrome.runtime.lastError || !reply) {
        resolve({ ok: false, error: "Chưa kết nối được với Card English. Hãy mở app Card English một lần để app tự đăng ký với trình duyệt, rồi thử lại." });
      } else {
        resolve(reply);
      }
    });
  });
}

chrome.contextMenus.onClicked.addListener(async (info, tab) => {
  if (info.menuItemId !== MENU) return;
  const text = (info.selectionText || "").trim();
  if (!text) return;
  const target = tab?.id >= 0 ? { tabId: tab.id, frameIds: [info.frameId || 0] } : null;
  let sentence = "";
  try {
    if (target) sentence = (await chrome.scripting.executeScript({ target, func: sentenceOfSelection }))[0]?.result || "";
  } catch {
    // The browser's own pages and the PDF viewer cannot be read: the word goes alone.
  }
  const reply = await send({ text, sentence, url: info.pageUrl || tab?.url || "" });
  const word = text.length > 40 ? text.slice(0, 40) + "..." : text;
  const message = reply.ok ? `Đã gửi "${word}" tới Card English. Nếu app chưa mở, từ sẽ hiện khi bạn mở app.` : reply.error || "Card English không nhận được từ này, hãy thử lại.";
  try {
    if (target) await chrome.scripting.executeScript({ target, func: noteInPage, args: [message, !!reply.ok] });
  } catch {
    // No note on a page the browser keeps to itself.
  }
});
