/**
 * 前端主逻辑。
 *
 * 与后端通过 Tauri 的 invoke 通信，用到 5 个命令：
 *   list_entries          —— 取全部记录（含每次现算的验证码）
 *   add_entry             —— 新增
 *   delete_entry          —— 删除整条
 *   update_recovery_codes —— 覆盖某条的恢复码列表
 *   vault_location        —— 取数据文件路径，显示在底部
 *
 * ## 刷新策略（这里有个坑）
 * 验证码每秒都变，所以每秒要向后端要一次数据。但**不能每秒重建 DOM** ——
 * 那样会把用户展开的恢复码、折叠的分组每秒重置一次，还会造成滚动位置跳动。
 *
 * 做法是算一个「结构指纹」：只由持久化字段（服务/账号/密钥/恢复码/备注）
 * 和当前搜索词组成。指纹不变就只更新验证码文字与圆环；指纹变了才重建。
 */

const { invoke } = window.__TAURI__.core;

/** 必须与后端 totp.rs 的 PERIOD 保持一致，用于算圆环百分比 */
const PERIOD = 30;

/** 剩余秒数降到这两个值时，圆环变色提醒 */
const WARN_AT = 10;
const CRIT_AT = 5;

const listViewEl = document.getElementById("list-view");
const formViewEl = document.getElementById("form-view");
const listEl = document.getElementById("list");
const searchEl = document.getElementById("search");
const serviceListEl = document.getElementById("service-list");
const formErrorEl = document.getElementById("form-error");
const footerEl = document.getElementById("vault-path");

/** 折叠的分组名，存 localStorage 让重启后保持 */
const collapsedGroups = new Set(
  JSON.parse(localStorage.getItem("collapsedGroups") || "[]")
);

/** 已展开恢复码的条目 key */
const expandedRecovery = new Set();

/** 二次确认的等待时长（毫秒） */
const CONFIRM_MS = 3000;

/**
 * 正在等待二次确认的按钮标识。
 * 同一时间只允许一个 —— 点了别的按钮就取消前一个的确认态。
 * 条目删除标识形如 `entry\0服务\0账号`，恢复码删除形如 `code\0服务\0账号\0恢复码`。
 */
let pendingConfirm = null;

/** 触发确认态的按钮元素，用来区分「再点同一个按钮」和「点别处」 */
let confirmAnchor = null;

let confirmTimer = null;

/**
 * 确认态的过期时刻（Unix 毫秒）。
 *
 * 为什么不只靠 setTimeout：Chromium 会节流被遮挡或失焦窗口的定时器，
 * 可能延迟几十秒才触发，表现成「确认按钮一直不消失」。
 * 所以这里记下绝对时刻，由每秒的 refresh() 兜底检查。
 */
let confirmDeadline = 0;

/** 每条记录的验证码/圆环元素引用，key -> {codeEl, ringEl, numEl} */
let rowRefs = new Map();

/**
 * 表单当前是「新增」还是「编辑」。
 * 为 null 表示新增；否则存的是被编辑记录**改之前**的服务名与账号 ——
 * 定位记录必须用原值，因为服务名和账号本身也可能被改。
 */
let editingOriginal = null;

/** 上一次的结构指纹 */
let lastKey = "";

let searchTerm = "";
let allEntries = [];
let vaultPathText = "";
let toastTimer = null;

/* ------------------------------------------------------------------ */
/* 小工具                                                              */
/* ------------------------------------------------------------------ */

/** 记录的唯一标识：服务 + 账号。后端删除也是用这两个字段定位。 */
const keyOf = (e) => `${e.label}\u0000${e.account}`;

/** 6 位验证码中间加个空格，照着输入时不容易串行 */
const formatCode = (c) =>
  c && c.length === 6 ? `${c.slice(0, 3)} ${c.slice(3)}` : c;

/** 按剩余秒数决定圆环颜色档位 */
function ringClass(remaining) {
  if (remaining <= CRIT_AT) return "ring crit";
  if (remaining <= WARN_AT) return "ring low";
  return "ring";
}

/** 在底部短暂显示一条提示，之后恢复成数据文件路径 */
function toast(msg, isError = true) {
  clearTimeout(toastTimer);
  footerEl.textContent = msg;
  footerEl.style.color = isError ? "var(--danger)" : "var(--ok)";
  toastTimer = setTimeout(() => {
    footerEl.textContent = vaultPathText;
    footerEl.style.color = "";
  }, 2500);
}

/* ------------------------------------------------------------------ */
/* 刷新                                                                */
/* ------------------------------------------------------------------ */

async function refresh() {
  // 确认态过期兜底：即使 setTimeout 被浏览器节流没触发，
  // 每秒一次的刷新也会在这里把它清掉。
  if (pendingConfirm && Date.now() >= confirmDeadline) {
    cancelConfirm();
  }

  try {
    allEntries = await invoke("list_entries");
  } catch (e) {
    listEl.replaceChildren(buildEmpty(`读取失败：${e}`, true));
    lastKey = "";
    return;
  }

  syncServiceOptions(allEntries);

  const shown = filterEntries(allEntries);
  const key =
    JSON.stringify(
      allEntries.map((e) => [e.label, e.account, e.secret, e.codes, e.note])
    ) +
    "|" +
    searchTerm;

  if (key !== lastKey) {
    lastKey = key;
    rebuild(shown);
  } else {
    updateCodes(shown);
  }
}

/** 只更新会变的验证码文字与圆环，不动 DOM 结构 */
function updateCodes(entries) {
  for (const e of entries) {
    const ref = rowRefs.get(keyOf(e));
    if (!ref || !ref.codeEl) continue;

    // 正在显示「已复制」反馈时不要覆盖，否则反馈会瞬间消失
    if (ref.codeEl.classList.contains("copied")) continue;

    const text = formatCode(e.code);
    if (ref.codeEl.textContent !== text) ref.codeEl.textContent = text;

    ref.numEl.textContent = String(e.remaining);
    ref.ringEl.className = ringClass(e.remaining);
    ref.ringEl.style.setProperty("--p", Math.round((e.remaining / PERIOD) * 100));
  }
}

function filterEntries(entries) {
  if (!searchTerm) return entries;
  const q = searchTerm.toLowerCase();
  return entries.filter(
    (e) =>
      e.label.toLowerCase().includes(q) ||
      e.account.toLowerCase().includes(q) ||
      (e.note || "").toLowerCase().includes(q)
  );
}

/* ------------------------------------------------------------------ */
/* 渲染                                                                */
/* ------------------------------------------------------------------ */

function buildEmpty(text, isError = false) {
  const p = document.createElement("p");
  p.className = isError ? "empty is-error" : "empty";
  p.textContent = text;
  return p;
}

function rebuild(entries) {
  rowRefs = new Map();
  listEl.replaceChildren();

  if (entries.length === 0) {
    listEl.appendChild(
      buildEmpty(searchTerm ? "没有匹配的记录" : "还没有记录，点右上角「+ 添加」")
    );
    return;
  }

  // 按服务（label）分组
  const groups = new Map();
  for (const e of entries) {
    if (!groups.has(e.label)) groups.set(e.label, []);
    groups.get(e.label).push(e);
  }

  // 服务名按字母序：顺序稳定，不会因为用了一次就跳位
  const names = [...groups.keys()].sort((a, b) => a.localeCompare(b, "zh"));
  for (const name of names) {
    listEl.appendChild(buildGroup(name, groups.get(name)));
  }
}

function buildGroup(name, items) {
  const section = document.createElement("section");
  // 搜索时强制展开，否则命中了却藏在折叠的分组里，等于没搜
  section.className =
    collapsedGroups.has(name) && !searchTerm ? "group collapsed" : "group";

  const head = document.createElement("div");
  head.className = "group-head";

  const caret = document.createElement("span");
  caret.className = "caret";
  caret.textContent = "▼";

  const nameEl = document.createElement("span");
  nameEl.className = "group-name";
  nameEl.textContent = name;

  const countEl = document.createElement("span");
  countEl.className = "group-count";
  countEl.textContent = String(items.length);

  head.append(caret, nameEl, countEl);
  head.onclick = () => {
    const nowCollapsed = section.classList.toggle("collapsed");
    if (nowCollapsed) collapsedGroups.add(name);
    else collapsedGroups.delete(name);
    localStorage.setItem(
      "collapsedGroups",
      JSON.stringify([...collapsedGroups])
    );
  };

  const body = document.createElement("div");
  body.className = "group-body";
  for (const e of items) body.appendChild(buildCard(e));

  section.append(head, body);
  return section;
}

function buildCard(e) {
  const key = keyOf(e);
  const card = document.createElement("div");
  card.className = "card";

  /* ---- 主行：账号 / 验证码 / 圆环 / 删除 ---- */
  const main = document.createElement("div");
  main.className = "card-main";

  const meta = document.createElement("div");
  meta.className = "meta";
  const accountEl = document.createElement("div");
  accountEl.className = "account";
  accountEl.textContent = e.account;
  meta.appendChild(accountEl);
  if (e.note) {
    const noteEl = document.createElement("div");
    noteEl.className = "note";
    noteEl.textContent = e.note;
    meta.appendChild(noteEl);
  }
  main.appendChild(meta);

  if (e.code) {
    const codeEl = document.createElement("div");
    codeEl.className = "code";
    codeEl.textContent = formatCode(e.code);
    codeEl.title = "点击复制";
    codeEl.onclick = () => copyText(e.code, codeEl, formatCode(e.code));
    main.appendChild(codeEl);

    const ringEl = document.createElement("div");
    ringEl.className = ringClass(e.remaining);
    ringEl.style.setProperty("--p", Math.round((e.remaining / PERIOD) * 100));
    const numEl = document.createElement("span");
    numEl.textContent = String(e.remaining);
    ringEl.appendChild(numEl);
    main.appendChild(ringEl);

    rowRefs.set(key, { codeEl, ringEl, numEl });
  } else if (e.error) {
    const errEl = document.createElement("div");
    errEl.className = "err";
    errEl.textContent = e.error;
    main.appendChild(errEl);
  }

  main.appendChild(buildActions(e));
  card.appendChild(main);

  /* ---- 恢复码折叠区 ---- */
  const codes = e.codes || [];
  if (codes.length > 0) {
    card.append(buildRecoverySection(e, codes, key));
  }

  return card;
}

/**
 * 取消当前的确认态（如果有）。
 *
 * 三种情况都会走到这里：等超时、点了别处、按了 Esc。
 */
function cancelConfirm() {
  if (!pendingConfirm) return;

  clearTimeout(confirmTimer);
  confirmTimer = null;
  pendingConfirm = null;
  confirmAnchor = null;
  confirmDeadline = 0;

  // 重置指纹以触发一次重建，把按钮恢复成普通的 ×
  lastKey = "";
  refresh();
}

/**
 * 两步确认：第一次点击让按钮进入确认态，3 秒内再点一次才真正执行。
 *
 * 删除是不可逆的 —— 记录删了要重新录入密钥，恢复码删了更是不可再生，
 * 所以两者都要确认。
 *
 * 三种方式可以退出确认态：再点一次（执行）、点别处、等待超时。
 *
 * @param id    确认态标识，同 id 视为同一个按钮
 * @param btn   触发确认的按钮元素，用于判定「点别处」
 * @param action 确认后要执行的动作
 */
async function twoStepConfirm(id, btn, action) {
  if (pendingConfirm !== id) {
    // 第一次点击：进入确认态
    clearTimeout(confirmTimer);
    pendingConfirm = id;
    confirmAnchor = btn;
    confirmDeadline = Date.now() + CONFIRM_MS;
    confirmTimer = setTimeout(cancelConfirm, CONFIRM_MS);

    lastKey = "";
    refresh();
    return;
  }

  // 第二次点击：执行
  clearTimeout(confirmTimer);
  confirmTimer = null;
  pendingConfirm = null;
  confirmAnchor = null;
  confirmDeadline = 0;

  try {
    await action();
  } catch (e) {
    toast(`操作失败：${e}`);
  }
  lastKey = "";
  refresh();
}

// 点空白处取消确认态。
// 注意要跳过「触发确认的那一下点击」—— 此时事件还在冒泡，按钮又还没重建出
// .confirm 类，靠 confirmAnchor 判断最可靠。
document.addEventListener("click", (ev) => {
  if (!pendingConfirm) return;
  if (confirmAnchor && (confirmAnchor === ev.target || confirmAnchor.contains(ev.target))) {
    return;
  }
  cancelConfirm();
});

// Esc 也取消
document.addEventListener("keydown", (ev) => {
  if (ev.key === "Escape" && pendingConfirm) {
    cancelConfirm();
  }
});

/** 卡片右侧的「编辑」+「删除」按钮组，平时透明，悬停卡片才显现 */
function buildActions(e) {
  const box = document.createElement("div");
  box.className = "actions";

  const edit = document.createElement("button");
  edit.className = "act";
  edit.textContent = "✎";
  edit.title = "编辑";
  edit.onclick = (ev) => {
    ev.stopPropagation();
    openForm(e);
  };

  box.append(edit, buildDeleteButton(e));
  return box;
}

/** 删除整条记录：首次点击进入确认态，3 秒内再点一次才真删 */
function buildDeleteButton(e) {
  const id = `entry\u0000${keyOf(e)}`;
  const btn = document.createElement("button");
  btn.className = "act del";
  btn.textContent = "×";
  btn.title = "删除";

  if (pendingConfirm === id) {
    btn.classList.add("confirm");
    btn.textContent = "确认删除";
  }

  btn.onclick = (ev) => {
    ev.stopPropagation();
    twoStepConfirm(id, btn, () =>
      invoke("delete_entry", { label: e.label, account: e.account })
    );
  };

  return btn;
}

function buildRecoverySection(entry, codes, key) {
  const foot = document.createElement("div");
  foot.className = "card-foot";

  const icon = document.createElement("span");
  icon.textContent = "🔑";

  const labelEl = document.createElement("span");
  labelEl.textContent = `恢复码 · ${codes.length} 个`;

  const caret = document.createElement("span");
  caret.className = "caret";
  caret.textContent = "▼";

  foot.append(icon, labelEl, caret);

  const box = document.createElement("div");
  box.className = "recovery";

  const hint = document.createElement("div");
  hint.className = "hint";
  hint.textContent = "一次性使用，用掉请删除";
  box.appendChild(hint);

  for (const code of codes) box.appendChild(buildRecoveryRow(entry, code));

  if (expandedRecovery.has(key)) {
    foot.classList.add("open");
    box.classList.add("open");
  }

  foot.onclick = () => {
    const open = box.classList.toggle("open");
    foot.classList.toggle("open", open);
    if (open) expandedRecovery.add(key);
    else expandedRecovery.delete(key);
  };

  const frag = document.createDocumentFragment();
  frag.append(foot, box);
  return frag;
}

function buildRecoveryRow(entry, code) {
  const id = `code\u0000${keyOf(entry)}\u0000${code}`;
  const row = document.createElement("div");
  row.className = "rc-row";

  const codeEl = document.createElement("code");
  codeEl.textContent = code;

  const copyEl = document.createElement("span");
  copyEl.className = "rc-copy";
  copyEl.textContent = "复制";
  copyEl.onclick = () => copyText(code, copyEl, "复制");

  const delEl = document.createElement("button");
  delEl.className = "rc-del";
  delEl.title = "删除这条恢复码";

  if (pendingConfirm === id) {
    delEl.classList.add("confirm");
    delEl.textContent = "确认删除";
  } else {
    delEl.textContent = "×";
  }

  delEl.onclick = () => {
    // 后端是整体覆盖语义，所以把「剩下的那些」传过去
    const rest = (entry.codes || []).filter((c) => c !== code);
    twoStepConfirm(id, delEl, () =>
      invoke("update_recovery_codes", {
        label: entry.label,
        account: entry.account,
        codes: rest.length ? rest : null,
      })
    );
  };

  row.append(codeEl, copyEl, delEl);
  return row;
}

/* ------------------------------------------------------------------ */
/* 复制                                                                */
/* ------------------------------------------------------------------ */

/**
 * 写剪贴板。
 *
 * 先试标准的 navigator.clipboard；Tauri 用自定义协议加载页面，
 * 若该环境下不被视为安全上下文，就回退到已废弃但可用的 execCommand。
 */
async function writeClipboard(text) {
  try {
    if (navigator.clipboard && window.isSecureContext) {
      await navigator.clipboard.writeText(text);
      return true;
    }
  } catch {
    // 落到下面的回退方案
  }

  try {
    const ta = document.createElement("textarea");
    ta.value = text;
    ta.style.position = "fixed";
    ta.style.opacity = "0";
    document.body.appendChild(ta);
    ta.select();
    const ok = document.execCommand("copy");
    document.body.removeChild(ta);
    return ok;
  } catch {
    return false;
  }
}

/**
 * 复制并在元素上显示短暂反馈。
 * 复制的是**去掉空格的原始验证码**，不是显示用的带空格版本。
 */
async function copyText(text, el, restoreTo) {
  const ok = await writeClipboard(text);
  if (!ok) {
    toast("复制失败");
    return;
  }

  const isCode = el.classList.contains("code");
  el.textContent = "已复制";
  if (isCode) el.classList.add("copied");
  else el.style.color = "var(--ok)";

  setTimeout(() => {
    el.textContent = restoreTo;
    if (isCode) el.classList.remove("copied");
    else el.style.color = "";
  }, 700);
}

/* ------------------------------------------------------------------ */
/* 服务下拉选项：从已有记录里取服务名，去重后排序                      */
/* ------------------------------------------------------------------ */

function syncServiceOptions(entries) {
  const names = [...new Set(entries.map((e) => e.label))].sort((a, b) =>
    a.localeCompare(b, "zh")
  );
  serviceListEl.replaceChildren();
  for (const n of names) {
    const opt = document.createElement("option");
    opt.value = n;
    serviceListEl.appendChild(opt);
  }
}

/* ------------------------------------------------------------------ */
/* 搜索                                                                */
/* ------------------------------------------------------------------ */

searchEl.oninput = () => {
  searchTerm = searchEl.value.trim();
  lastKey = ""; // 强制重建
  refresh();
};

/* ------------------------------------------------------------------ */
/* 新增表单                                                            */
/* ------------------------------------------------------------------ */

const FORM_FIELDS = ["f-service", "f-account", "f-secret", "f-codes", "f-note"];
const formTitleEl = document.getElementById("form-title");

/**
 * 打开表单。
 *
 * 传 `entry` 进入**编辑模式**（字段预填，保存时改原记录）；
 * 不传则是**新增模式**。
 *
 * 编辑模式下恢复码会按「一行一个」展开，这样既能删掉某行，
 * 也能在末尾补新的恢复码 —— 后端是整体覆盖语义，所以直接改文本就行。
 */
function openForm(entry = null) {
  // 定位记录要用「改之前」的值，因为服务名和账号本身也可能被改
  editingOriginal = entry
    ? { label: entry.label, account: entry.account }
    : null;

  formTitleEl.textContent = entry ? "编辑账号" : "新增账号";

  document.getElementById("f-service").value = entry ? entry.label : "";
  document.getElementById("f-account").value = entry ? entry.account : "";
  document.getElementById("f-secret").value = entry?.secret || "";
  document.getElementById("f-note").value = entry?.note || "";
  document.getElementById("f-codes").value = (entry?.codes || []).join("\n");

  listViewEl.style.display = "none";
  formViewEl.classList.add("open");
  formErrorEl.textContent = "";
  document.getElementById("f-service").focus();
}

function closeForm() {
  editingOriginal = null;
  formTitleEl.textContent = "新增账号";
  formViewEl.classList.remove("open");
  listViewEl.style.display = "flex";
  formErrorEl.textContent = "";
  for (const id of FORM_FIELDS) document.getElementById(id).value = "";
}

// 注意不能直接写 `= openForm`，那样会把点击事件对象当成 entry 传进去
document.getElementById("btn-add").onclick = () => openForm();
document.getElementById("btn-cancel").onclick = closeForm;

document.getElementById("btn-save").onclick = async () => {
  formErrorEl.textContent = "";

  const label = document.getElementById("f-service").value.trim();
  const account = document.getElementById("f-account").value.trim();
  const secret = document.getElementById("f-secret").value.trim();
  const note = document.getElementById("f-note").value.trim();
  const codes = document
    .getElementById("f-codes")
    .value.split(/\r?\n/)
    .map((s) => s.trim())
    .filter(Boolean);

  if (!label) {
    formErrorEl.textContent = "服务不能为空";
    return;
  }
  if (!account) {
    formErrorEl.textContent = "账号不能为空";
    return;
  }

  try {
    if (editingOriginal) {
      await invoke("update_entry", {
        originalLabel: editingOriginal.label,
        originalAccount: editingOriginal.account,
        entry: {
          label,
          account,
          secret: secret || null,
          codes: codes.length ? codes : null,
          note: note || null,
        },
      });
    } else {
      await invoke("add_entry", {
        label,
        account,
        secret: secret || null,
        codes: codes.length ? codes : null,
        note: note || null,
      });
    }
    closeForm();
    lastKey = "";
    refresh();
  } catch (e) {
    formErrorEl.textContent = String(e);
  }
};

/* ------------------------------------------------------------------ */
/* 主题                                                                */
/* ------------------------------------------------------------------ */

/**
 * 亮暗主题切换。
 *
 * 主题值只写在 <html data-theme="…"> 上，CSS 靠它整套换掉颜色变量，
 * 所以这里除了按钮上的图标之外不碰任何样式 —— 改一个属性换一整套配色。
 *
 * 选择记在 localStorage：选过就一直听用户的；没选过则跟随系统，
 * 并在系统设置变化时实时跟着变。首屏那一下由 index.html 里的内联脚本
 * 提前设好（module 脚本执行太晚，否则亮色用户会先看到一帧深色）。
 */
const THEME_KEY = "theme";
const themeBtn = document.getElementById("btn-theme");
const prefersLight = window.matchMedia("(prefers-color-scheme: light)");

/** 当前主题。data-theme 缺失或值意外时一律按暗色处理 */
const currentTheme = () =>
  document.documentElement.dataset.theme === "light" ? "light" : "dark";

function applyTheme(theme) {
  document.documentElement.dataset.theme = theme;
  // 按钮上显示的是「点下去会切到」的主题，不是当前主题
  themeBtn.textContent = theme === "light" ? "🌙" : "☀";
  themeBtn.title = theme === "light" ? "切换到暗色主题" : "切换到亮色主题";
}

themeBtn.onclick = () => {
  const next = currentTheme() === "light" ? "dark" : "light";
  localStorage.setItem(THEME_KEY, next);
  applyTheme(next);
};

// 没手动选过时，跟随系统设置的变化
prefersLight.addEventListener("change", (ev) => {
  if (!localStorage.getItem(THEME_KEY)) {
    applyTheme(ev.matches ? "light" : "dark");
  }
});

// 主题本身已由内联脚本设好，这里只是把按钮图标同步过去
applyTheme(currentTheme());

/* ------------------------------------------------------------------ */
/* 启动                                                                */
/* ------------------------------------------------------------------ */

(async () => {
  try {
    vaultPathText = await invoke("vault_location");
    footerEl.textContent = vaultPathText;
  } catch {
    // 取不到就不显示
  }

  await refresh();
  setInterval(refresh, 1000);
})();
