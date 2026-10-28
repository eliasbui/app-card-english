// Markdown for the tutor's answers. The HTML is escaped first and only known elements are built from the marks, so an
// answer can never put markup of its own on the page; links and images stay text.
(() => {
  "use strict";
  const escape = (s) => String(s ?? "").replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[c]));
  const LIST = /^\s*([-*+]|\d+[.)])\s+/;
  const ROW = /^\s*\|.*\|\s*$/;
  const RULE = /^\s*\|?\s*:?-+:?\s*(\|\s*:?-+:?\s*)*\|?\s*$/;

  /// Bold, italic and inline code within a line; `bold` gives the HTML of a bold run.
  const inline = (text, bold) => text
    .replace(/`([^`]+)`/g, "<code>$1</code>")
    .replace(/\*\*(.+?)\*\*/g, (_, t) => bold(t))
    .replace(/(^|[^*\w])\*(?!\s)([^*]+?)\*(?!\w)/g, "$1<em>$2</em>")
    .replace(/(^|[^_\w])_(?!\s)([^_]+?)_(?!\w)/g, "$1<em>$2</em>");

  /// `text` as HTML: headings, paragraphs and line breaks, lists nested one level, pipe tables, quotes, fenced blocks.
  function markdown(text, bold = (t) => `<b>${t}</b>`) {
    const lines = escape(text).replace(/\r/g, "").split("\n");
    const indent = (l) => l.match(/^\s*/)[0].length;
    const out = [];
    let i = 0;
    const list = () => {
      const base = indent(lines[i]), ordered = /^\s*\d/.test(lines[i]), items = [];
      while (i < lines.length && LIST.test(lines[i]) && indent(lines[i]) >= base) {
        if (indent(lines[i]) > base && items.length) {
          items[items.length - 1] += list();
          continue;
        }
        items.push(inline(lines[i++].replace(LIST, ""), bold));
        while (i < lines.length && lines[i].trim() && !LIST.test(lines[i]) && indent(lines[i]) > base) {
          items[items.length - 1] += "<br>" + inline(lines[i++].trim(), bold);
        }
      }
      const tag = ordered ? "ol" : "ul";
      return `<${tag}>${items.map((x) => `<li>${x}</li>`).join("")}</${tag}>`;
    };
    const cells = (l) => l.trim().replace(/^\||\|$/g, "").split("|").map((c) => inline(c.trim(), bold));
    while (i < lines.length) {
      const line = lines[i];
      if (/^\s*```/.test(line)) {
        const body = [];
        for (i++; i < lines.length && !/^\s*```/.test(lines[i]); i++) body.push(lines[i]);
        i++;
        out.push(`<pre>${body.join("\n")}</pre>`);
      } else if (!line.trim()) {
        i++;
      } else if (/^\s*#{1,6}\s/.test(line)) {
        out.push(`<p class="md-h">${inline(line.replace(/^\s*#{1,6}\s+/, ""), bold)}</p>`);
        i++;
      } else if (/^\s*&gt;/.test(line)) {
        const body = [];
        while (i < lines.length && /^\s*&gt;/.test(lines[i])) body.push(inline(lines[i++].replace(/^\s*&gt;\s?/, ""), bold));
        out.push(`<blockquote>${body.join("<br>")}</blockquote>`);
      } else if (ROW.test(line) && RULE.test(lines[i + 1] || "")) {
        const head = cells(line), rows = [];
        for (i += 2; i < lines.length && ROW.test(lines[i]); i++) rows.push(cells(lines[i]));
        out.push(`<table><thead><tr>${head.map((c) => `<th>${c}</th>`).join("")}</tr></thead><tbody>${
          rows.map((r) => `<tr>${r.map((c) => `<td>${c}</td>`).join("")}</tr>`).join("")}</tbody></table>`);
      } else if (LIST.test(line)) {
        out.push(list());
      } else {
        const body = [];
        while (i < lines.length && lines[i].trim() && !LIST.test(lines[i]) && !/^\s*(#{1,6}\s|```|&gt;)/.test(lines[i])) body.push(inline(lines[i++], bold));
        out.push(`<p>${body.join("<br>")}</p>`);
      }
    }
    return out.join("");
  }

  // `node ui/parts/chat-md.js` checks the renderer.
  if (typeof window === "undefined") {
    const eq = (a, b) => { if (a !== b) throw new Error(`\n${a}\n!==\n${b}`); };
    eq(markdown("<script>alert(1)<\/script>"), "<p>&lt;script&gt;alert(1)&lt;/script&gt;</p>");
    eq(markdown("- a\n- **b**"), "<ul><li>a</li><li><b>b</b></li></ul>");
    eq(markdown("1. x\n   - y\n2. z"), "<ol><li>x<ul><li>y</li></ul></li><li>z</li></ol>");
    eq(markdown("| a | b |\n|---|:-:|\n| 1 | *2* |"), "<table><thead><tr><th>a</th><th>b</th></tr></thead><tbody><tr><td>1</td><td><em>2</em></td></tr></tbody></table>");
    eq(markdown("## Ví dụ\nDòng một\ndòng hai\n\n> trích"), '<p class="md-h">Ví dụ</p><p>Dòng một<br>dòng hai</p><blockquote>trích</blockquote>');
    eq(markdown("```\n<b>x</b>\n```"), "<pre>&lt;b&gt;x&lt;/b&gt;</pre>");
    eq(markdown("dùng `take off` và **word**", (t) => `<b class="say">${t}</b>`), '<p>dùng <code>take off</code> và <b class="say">word</b></p>');
    eq(markdown("2 * 3 * 4 và snake_case_name"), "<p>2 * 3 * 4 và snake_case_name</p>");
    return console.log("chatMarkdown: ok");
  }
  window.chatMarkdown = markdown;
})();
