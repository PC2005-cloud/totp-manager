const { invoke } = window.__TAURI__.core;

const listEl = document.getElementById("list");
const formEl = document.getElementById("add-form");
const formErrorEl = document.getElementById("form-error");

// ---------- 渲染 ----------

async function refresh() {
  let entries;
  try {
    entries = await invoke("list_entries");
  } catch (e) {
    listEl.innerHTML = `<p class="empty">读取失败：${e}</p>`;
    return;
  }

  if (entries.length === 0) {
    listEl.innerHTML = `<p class="empty">还没有记录，点右上角「+ 添加」</p>`;
    return;
  }

  listEl.innerHTML = "";
  for (const e of entries) {
    listEl.appendChild(renderEntry(e));
  }
}

function renderEntry(e) {
  const row = document.createElement("div");
  row.className = "entry";

  const meta = document.createElement("div");
  meta.className = "meta";
  const label = document.createElement("div");
  label.className = "label";
  label.textContent = e.label;
  const account = document.createElement("div");
  account.className = "account";
  account.textContent = e.account;
  meta.append(label, account);

  const right = document.createElement("div");
  right.className = "right";

  if (e.code) {
    const code = document.createElement("span");
    code.className = "code";
    code.textContent = e.code;
    code.title = "点击复制";
    code.onclick = () => copyCode(code);
    right.appendChild(code);

    const remain = document.createElement("span");
    remain.className = "remaining";
    remain.textContent = `${e.remaining}s`;
    right.appendChild(remain);
  } else if (e.error) {
    const err = document.createElement("span");
    err.className = "err";
    err.textContent = e.error;
    right.appendChild(err);
  }

  const del = document.createElement("button");
  del.className = "del";
  del.textContent = "×";
  del.title = "删除";
  del.onclick = async () => {
    await invoke("delete_entry", { label: e.label, account: e.account });
    refresh();
  };
  right.appendChild(del);

  row.append(meta, right);
  return row;
}

async function copyCode(el) {
  try {
    await navigator.clipboard.writeText(el.textContent);
    const old = el.textContent;
    el.textContent = "已复制";
    setTimeout(() => (el.textContent = old), 600);
  } catch {
    // 剪贴板不可用时忽略
  }
}

// ---------- 添加 ----------

document.getElementById("add-toggle").onclick = () => {
  formEl.classList.toggle("hidden");
  formErrorEl.textContent = "";
};

document.getElementById("add-cancel").onclick = () => {
  formEl.classList.add("hidden");
  formEl.reset();
  formErrorEl.textContent = "";
};

formEl.onsubmit = async (ev) => {
  ev.preventDefault();
  formErrorEl.textContent = "";
  try {
    await invoke("add_entry", {
      label: document.getElementById("f-label").value,
      account: document.getElementById("f-account").value,
      secret: document.getElementById("f-secret").value || null,
      note: document.getElementById("f-note").value || null,
    });
    formEl.reset();
    formEl.classList.add("hidden");
    refresh();
  } catch (e) {
    formErrorEl.textContent = e;
  }
};

// ---------- 启动 ----------

(async () => {
  try {
    document.getElementById("vault-path").textContent = await invoke("vault_location");
  } catch {
    // 忽略
  }
  refresh();

  // 每秒刷新验证码与倒计时
  setInterval(refresh, 1000);
})();
