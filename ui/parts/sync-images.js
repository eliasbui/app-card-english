// Pictures on the cards: shown wherever a word's details are, added in the editor and in the word's window by
// pasting, dropping or choosing a file. Each is scaled down here before it goes to Rust, which keeps the file.
(() => {
  "use strict";
  /// The long side of a picture, in pixels.
  const SIDE = 512;
  const PASTE = /Mac/.test(navigator.platform) ? "Cmd+V" : "Ctrl+V";

  // Pictures come from Rust as data: URLs (the page's CSP takes no other), asked once and kept for a while.
  const cache = new Map();
  function picture(name) {
    if (!cache.has(name)) {
      cache.set(name, app.call("image", { name }));
      if (cache.size > 60) cache.delete(cache.keys().next().value);
    }
    return cache.get(name);
  }
  // Every <img data-img> the page draws gets its picture; one whose file is not on this machine stays hidden.
  let queued = false;
  function fill() {
    queued = false;
    document.querySelectorAll("img[data-img]:not([data-asked])").forEach((img) => {
      const name = img.dataset.img;
      img.dataset.asked = "1";
      picture(name).then((url) => { if (img.dataset.img === name) img.src = url; }, () => cache.delete(name));
    });
  }
  new MutationObserver(() => {
    if (!queued) { queued = true; setTimeout(fill, 0); }
  }).observe(document.body, { childList: true, subtree: true });

  /// A picture file scaled down to SIDE on its long side, as WebP, or JPEG where the web view cannot write WebP.
  function shrink(file) {
    return new Promise((resolve, reject) => {
      if (!file || !/^image\//.test(file.type)) return reject("Hãy chọn một file ảnh");
      const reader = new FileReader();
      reader.onerror = () => reject("Không đọc được file ảnh này");
      reader.onload = () => {
        const img = new Image();
        img.onerror = () => reject("Không mở được ảnh này, hãy thử ảnh JPEG, PNG hoặc WebP");
        img.onload = () => {
          try {
            const scale = Math.min(1, SIDE / Math.max(img.naturalWidth, img.naturalHeight, 1));
            const canvas = document.createElement("canvas");
            canvas.width = Math.max(1, Math.round(img.naturalWidth * scale));
            canvas.height = Math.max(1, Math.round(img.naturalHeight * scale));
            const ctx = canvas.getContext("2d");
            ctx.imageSmoothingQuality = "high";
            ctx.drawImage(img, 0, 0, canvas.width, canvas.height);
            let url = canvas.toDataURL("image/webp", 0.82);
            if (!url.startsWith("data:image/webp")) {
              // JPEG has no transparency: what was see-through turns white rather than black.
              ctx.globalCompositeOperation = "destination-over";
              ctx.fillStyle = "#fff";
              ctx.fillRect(0, 0, canvas.width, canvas.height);
              url = canvas.toDataURL("image/jpeg", 0.85);
            }
            resolve(url);
          } catch {
            reject("Không mở được ảnh này, hãy thử ảnh JPEG, PNG hoặc WebP");
          }
        };
        img.src = reader.result;
      };
      reader.readAsDataURL(file);
    });
  }
  const imageIn = (list) => [...(list || [])].find((f) => /^image\//.test(f.type)) || null;
  const imageOf = (data) => imageIn(data && data.files) || imageIn([...((data && data.items) || [])].filter((i) => i.kind === "file").map((i) => i.getAsFile()));
  const hasFiles = (e) => !!e.dataTransfer && [...e.dataTransfer.types].includes("Files");

  // One file chooser for the editor and the word's window.
  document.body.insertAdjacentHTML("beforeend", '<input type="file" id="sync-file" accept="image/*" hidden>');
  let chosen = null;
  $("sync-file").addEventListener("change", (e) => {
    const file = e.target.files[0];
    e.target.value = "";
    if (file && chosen) chosen(file);
  });
  const choose = (take) => { chosen = take; $("sync-file").click(); };

  // The editor: the picture waits in the word being edited until Lưu sends it with the rest (`image_data`), or
  // an empty `image` when it was removed.
  function editorBox() {
    if ($("sync-drop")) return;
    $("f-pos").insertAdjacentHTML("afterend", `
      <label>Hình ảnh</label>
      <div class="sync-drop" id="sync-drop" tabindex="0" aria-label="Hình ảnh của từ: bấm vào rồi dán, kéo thả hoặc chọn file">
        <img id="sync-drop-img" class="hidden" alt="Hình minh họa của từ">
        <div class="grow"><div class="muted small" id="sync-drop-hint"></div>
          <div class="row"><button type="button" class="btn small" id="sync-pick"></button>
          <button type="button" class="btn small ghost danger" id="sync-remove">Bỏ ảnh</button></div></div>
      </div>`);
    $("sync-pick").onclick = () => choose(setEditorImage);
    $("sync-remove").onclick = () => {
      delete state.editing.image_data;
      state.editing.image = "";
      showEditor();
    };
  }
  function showEditor() {
    const w = state.editing;
    if (!w || !$("sync-drop")) return;
    const img = $("sync-drop-img"), has = !!(w.image_data || w.image);
    img.classList.toggle("hidden", !has);
    delete img.dataset.asked;
    if (w.image_data) {
      img.removeAttribute("data-img");
      img.src = w.image_data;
    } else if (w.image) {
      img.removeAttribute("src");
      img.dataset.img = w.image;
      fill();
    }
    $("sync-drop-hint").textContent = has ? "Dán hoặc kéo thả ảnh khác vào để thay." : `Bấm vào ô này rồi dán ảnh (${PASTE}), kéo thả ảnh vào, hoặc chọn file.`;
    $("sync-pick").textContent = has ? "Đổi ảnh" : "Chọn ảnh";
    $("sync-remove").classList.toggle("hidden", !has);
  }
  async function setEditorImage(file) {
    if (!state.editing) return;
    try {
      state.editing.image_data = await shrink(file);
      $("editor-error").textContent = "";
      showEditor();
    } catch (e) {
      $("editor-error").textContent = String(e);
    }
  }
  hooks.editor.push((w) => {
    editorBox();
    // A copy, so that a picture chosen and then cancelled never sticks to the word in the list.
    state.editing = { ...w };
    delete state.editing.image_data;
    showEditor();
  });

  // The word's window: a small picture beside the word, and buttons that change it at once.
  let viewed = null;
  hooks.word.push((w) => {
    viewed = w;
    const box = $("word-detail");
    if (w.image) box.querySelector(".wd-head").insertAdjacentHTML("afterbegin", `<img class="sync-thumb" data-img="${esc(w.image)}" alt="">`);
    box.querySelector(".wd-actions").insertAdjacentHTML("beforebegin", `
      <div class="row sync-img-tools">
        <button class="btn small" data-img-act="pick">${w.image ? "Đổi ảnh" : "Thêm ảnh"}</button>
        ${w.image ? '<button class="btn small ghost danger" data-img-act="remove">Bỏ ảnh</button>' : ""}
        <span class="muted small">hoặc dán (${PASTE}), kéo thả ảnh vào đây</span>
      </div>`);
  });
  $("word-detail").addEventListener("click", (e) => {
    const b = e.target.closest("[data-img-act]");
    if (!b) return;
    if (b.dataset.imgAct === "pick") choose(setWordImage);
    if (b.dataset.imgAct === "remove") setWordImage(null);
  });
  async function setWordImage(file) {
    const w = viewed;
    if (!w || $("word-modal").classList.contains("hidden")) return;
    try {
      const data = file ? await shrink(file) : "";
      Object.assign(w, await app.call("set_image", { id: w.id, data }));
      toast(file ? `Đã thêm ảnh cho "${w.word}"` : `Đã bỏ ảnh của "${w.word}"`);
      if (viewed === w) openWord(w.id);
    } catch (e) {
      toast(String(e));
    }
  }

  // Pasting goes to the word's window when it is open, else to the editor; text pasted into a field stays text.
  document.addEventListener("paste", (e) => {
    const file = imageOf(e.clipboardData);
    if (!file || (e.target.matches?.("input, textarea") && e.clipboardData.types.includes("text/plain"))) return;
    if (!$("word-modal").classList.contains("hidden")) {
      e.preventDefault();
      setWordImage(file);
    } else if (state.editing && $("page-words").classList.contains("active")) {
      e.preventDefault();
      setEditorImage(file);
    }
  });
  for (const [zone, take] of [[$("editor"), setEditorImage], [$("word-detail"), setWordImage]]) {
    zone.addEventListener("dragover", (e) => {
      if (!hasFiles(e)) return;
      e.preventDefault();
      zone.classList.add("sync-over");
    });
    zone.addEventListener("dragleave", (e) => { if (!zone.contains(e.relatedTarget)) zone.classList.remove("sync-over"); });
    zone.addEventListener("drop", (e) => {
      zone.classList.remove("sync-over");
      const file = hasFiles(e) && imageIn(e.dataTransfer.files);
      if (!file) return;
      e.preventDefault();
      e.stopPropagation();
      take(file);
    });
  }
  // A file dropped anywhere else must not replace the page with the file.
  document.addEventListener("dragover", (e) => { if (hasFiles(e)) e.preventDefault(); });
  document.addEventListener("drop", (e) => { if (hasFiles(e)) e.preventDefault(); });
})();
