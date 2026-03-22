import { invoke }      from "@tauri-apps/api/core";
import { listen }       from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";

// ── Step definitions ────────────────────────────────────────────────────────
const STEPS = [
  { id: "welcome",  label: "Welcome"    },
  { id: "license",  label: "License"    },
  { id: "options",  label: "Options"    },
  { id: "install",  label: "Installing" },
  { id: "done",     label: "Complete"   },
];

// ── State ───────────────────────────────────────────────────────────────────
let currentStep = 0;
let licenseAgreed = false;
let isAdmin = false;

// Ordered install-step rows to track in the UI
const INSTALL_ROWS = [
  { key: "directory",  label: "Creating installation directory" },
  { key: "copy_core",  label: "Copying VeloceCore service"      },
  { key: "copy_dash",  label: "Copying VeloceNetwork Dashboard"  },
  { key: "service",    label: "Registering Windows service"      },
  { key: "shortcuts",  label: "Creating shortcuts"               },
  { key: "path",       label: "Updating system PATH"             },
  { key: "registry",   label: "Registering uninstaller"          },
  { key: "nrpt",       label: "Configuring .vln DNS routing"     },
];

// ── Titlebar wiring ─────────────────────────────────────────────────────────
const win = getCurrentWindow();
document.getElementById("btn-close").addEventListener("click",    () => win.close());
document.getElementById("btn-minimize").addEventListener("click", () => win.minimize());

// ── Sidebar rendering ────────────────────────────────────────────────────────
function renderSidebar() {
  const nav = document.getElementById("step-nav");
  nav.innerHTML = STEPS.map((s, i) => {
    let cls = "upcoming";
    if (i === currentStep) cls = "active";
    else if (i < currentStep) cls = "done";

    const bubble = i < currentStep
      ? `<div class="step-bubble">✓</div>`
      : `<div class="step-bubble">${i + 1}</div>`;

    return `<div class="nav-step ${cls}" data-step="${i}">
      ${bubble}
      <span class="step-label">${s.label}</span>
    </div>`;
  }).join(`<div style="height:12px"></div>`);
}

// ── Step renderers ───────────────────────────────────────────────────────────

function renderWelcome() {
  return `
  <div class="step active" id="step-welcome">
    <div class="step-title">Welcome to VeloceNetwork</div>
    <div class="step-subtitle">This wizard will install VeloceNetwork on your machine.</div>
    <div class="step-body">
      <div class="welcome-hero">
        <div class="hero-icon">⚡</div>
        <span class="hero-badge">✦ v1.1.0 Release</span>
      </div>
      <div class="feature-grid">
        <div class="feature-card">
          <span class="feat-icon">🔧</span>
          <div class="feat-text">
            <h4>VeloceCore Service</h4>
            <p>Background service managing node processes with job isolation.</p>
          </div>
        </div>
        <div class="feature-card">
          <span class="feat-icon">📊</span>
          <div class="feat-text">
            <h4>Dashboard</h4>
            <p>Real-time Tauri UI for spawning, monitoring, and killing nodes.</p>
          </div>
        </div>
        <div class="feature-card">
          <span class="feat-icon">🌐</span>
          <div class="feat-text">
            <h4>VeloceNet</h4>
            <p>Private <code>*.vln</code> DNS namespace with SOCKS5 proxy.</p>
          </div>
        </div>
        <div class="feature-card">
          <span class="feat-icon">🔒</span>
          <div class="feat-text">
            <h4>Secure IPC</h4>
            <p>Named-pipe ACL with SID-based auth and per-session PSK.</p>
          </div>
        </div>
      </div>
      <div class="admin-warn ${isAdmin ? "" : "visible"}" id="admin-warn">
        ⚠ Running without Administrator privileges. Service installation may fail.
      </div>
    </div>
    <div class="step-footer">
      <button class="btn btn-primary" id="btn-next-welcome">Next →</button>
    </div>
  </div>`;
}

const LICENSE_TEXT = `VeloceNetwork — Proprietary Software License

Copyright (c) 2025 VeloceSolutions. All rights reserved.

IMPORTANT — READ CAREFULLY: This End-User License Agreement ("EULA")
is a legal agreement between you ("User") and VeloceSolutions for the
VeloceNetwork software product ("Software").

1. GRANT OF LICENSE
   Subject to the terms of this EULA, VeloceSolutions grants you a
   non-exclusive, non-transferable, limited license to install and use
   the Software solely for your internal business purposes.

2. RESTRICTIONS
   You may not: (a) copy or duplicate the Software; (b) modify,
   translate, adapt, or create derivative works; (c) reverse engineer,
   disassemble, or decompile the Software; (d) rent, lease, lend,
   sell, redistribute, or sublicense the Software.

3. OWNERSHIP
   VeloceSolutions retains all title, ownership, and intellectual
   property rights in and to the Software.

4. DISCLAIMER OF WARRANTIES
   THE SOFTWARE IS PROVIDED "AS IS" WITHOUT WARRANTY OF ANY KIND.
   VELOCESOLUTIONS DISCLAIMS ALL WARRANTIES, EXPRESS OR IMPLIED,
   INCLUDING WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A
   PARTICULAR PURPOSE.

5. LIMITATION OF LIABILITY
   IN NO EVENT SHALL VELOCESOLUTIONS BE LIABLE FOR ANY INDIRECT,
   INCIDENTAL, SPECIAL, OR CONSEQUENTIAL DAMAGES ARISING FROM THE USE
   OR INABILITY TO USE THE SOFTWARE.

6. TERMINATION
   This EULA terminates automatically if you breach any term. Upon
   termination you must destroy all copies of the Software.

By clicking "I agree" you acknowledge that you have read, understood,
and agree to be bound by the terms of this EULA.`;

function renderLicense() {
  return `
  <div class="step" id="step-license">
    <div class="step-title">License Agreement</div>
    <div class="step-subtitle">Please read and accept the license to continue.</div>
    <div class="step-body">
      <div class="license-box">${LICENSE_TEXT}</div>
      <label class="license-agree" for="chk-agree">
        <input type="checkbox" id="chk-agree" ${licenseAgreed ? "checked" : ""} />
        <span>I have read and agree to the License Agreement</span>
      </label>
    </div>
    <div class="step-footer">
      <button class="btn btn-ghost"   id="btn-back-license">← Back</button>
      <button class="btn btn-primary" id="btn-next-license" ${licenseAgreed ? "" : "disabled"}>Next →</button>
    </div>
  </div>`;
}

async function renderOptions() {
  const defaultDir = await invoke("get_default_dir").catch(() => "C:\\Program Files\\VeloceNetwork");
  return `
  <div class="step" id="step-options">
    <div class="step-title">Installation Options</div>
    <div class="step-subtitle">Choose where and how to install VeloceNetwork.</div>
    <div class="step-body">
      <div class="option-group">
        <div class="section-label">Installation Directory</div>
        <div class="dir-row">
          <input class="dir-input" type="text" id="install-dir" value="${escHtml(defaultDir)}" spellcheck="false" />
          <button class="btn btn-ghost" id="btn-browse">Browse…</button>
        </div>

        <div class="section-label">What to Install</div>
        <label class="option-row" for="chk-install-core">
          <input type="checkbox" id="chk-install-core" checked />
          <span class="option-icon">🔧</span>
          <div class="option-text"><h4>VeloceCore Service</h4><span>Background service that manages nodes &amp; DNS</span></div>
        </label>
        <label class="option-row" for="chk-install-dashboard">
          <input type="checkbox" id="chk-install-dashboard" checked />
          <span class="option-icon">📊</span>
          <div class="option-text"><h4>Dashboard</h4><span>Tauri GUI for monitoring and control</span></div>
        </label>
        <label class="option-row" for="chk-install-cli">
          <input type="checkbox" id="chk-install-cli" checked />
          <span class="option-icon">💻</span>
          <div class="option-text"><h4>CLI Tools</h4><span>veloce-run, veloce-launcher (startup menu), veloce-shell</span></div>
        </label>

        <div class="section-label">Components</div>
        <label class="option-row" for="chk-service">
          <input type="checkbox" id="chk-service" checked />
          <span class="option-icon">🔧</span>
          <div class="option-text"><h4>Start service after install</h4><span>Launches VeloceCore immediately</span></div>
        </label>
        <label class="option-row" for="chk-path">
          <input type="checkbox" id="chk-path" checked />
          <span class="option-icon">🛤</span>
          <div class="option-text"><h4>Add to system PATH</h4><span>Use veloce CLI from any terminal</span></div>
        </label>

        <div class="section-label">Shortcuts</div>
        <label class="option-row" for="chk-startmenu">
          <input type="checkbox" id="chk-startmenu" checked />
          <span class="option-icon">📌</span>
          <div class="option-text"><h4>Start Menu shortcut</h4><span>Pin VeloceNetwork in Start Menu</span></div>
        </label>
        <label class="option-row" for="chk-desktop">
          <input type="checkbox" id="chk-desktop" />
          <span class="option-icon">🖥</span>
          <div class="option-text"><h4>Desktop shortcut</h4><span>Create shortcut on the Desktop</span></div>
        </label>
      </div>
    </div>
    <div class="step-footer">
      <button class="btn btn-ghost"   id="btn-back-options">← Back</button>
      <button class="btn btn-primary" id="btn-install">Install</button>
    </div>
  </div>`;
}

function renderInstalling() {
  const rows = INSTALL_ROWS.map(r => `
    <div class="istep" id="istep-${r.key}">
      <div class="istep-icon" id="istep-icon-${r.key}">○</div>
      <span class="istep-label">${r.label}</span>
    </div>
  `).join("");

  return `
  <div class="step" id="step-install">
    <div class="step-title">Installing…</div>
    <div class="step-subtitle">Please wait while VeloceNetwork is being installed.</div>
    <div class="step-body">
      <div class="progress-track">
        <div class="progress-fill" id="progress-fill"></div>
      </div>
      <div class="install-steps">${rows}</div>
    </div>
    <div class="step-footer">
      <button class="btn btn-primary" id="btn-next-install" disabled>Next →</button>
    </div>
  </div>`;
}

function renderDone() {
  return `
  <div class="step" id="step-done">
    <div class="step-title">All Done!</div>
    <div class="step-subtitle">VeloceNetwork has been successfully installed.</div>
    <div class="step-body">
      <div class="done-hero">
        <div class="done-check">✓</div>
        <div class="done-title">Installation Complete</div>
        <div class="done-subtitle">VeloceNetwork is ready on your machine.</div>
        <div class="done-options">
          <label class="launch-row" for="chk-launch">
            <input type="checkbox" id="chk-launch" checked />
            <label for="chk-launch">Launch VeloceNetwork Dashboard now</label>
          </label>
        </div>
      </div>
    </div>
    <div class="step-footer">
      <button class="btn btn-primary" id="btn-finish">Finish</button>
    </div>
  </div>`;
}

// ── Step navigation ──────────────────────────────────────────────────────────

async function showStep(idx) {
  currentStep = idx;
  renderSidebar();

  const panel = document.getElementById("content-panel");

  let html;
  switch (idx) {
    case 0: html = renderWelcome();         break;
    case 1: html = renderLicense();         break;
    case 2: html = await renderOptions();   break;
    case 3: html = renderInstalling();      break;
    case 4: html = renderDone();            break;
    default: return;
  }

  panel.innerHTML = html;

  // Mark step active (needed for CSS animation)
  const stepEl = panel.querySelector(".step");
  if (stepEl) stepEl.classList.add("active");

  wireStep(idx);
}

function wireStep(idx) {
  switch (idx) {
    case 0: wireWelcome();          break;
    case 1: wireLicense();          break;
    case 2: wireOptions();          break;
    case 3: /* wired by install */  break;
    case 4: wireDone();             break;
  }
}

// ── Wiring per step ──────────────────────────────────────────────────────────

function wireWelcome() {
  document.getElementById("btn-next-welcome").addEventListener("click", () => showStep(1));
}

function wireLicense() {
  const chk     = document.getElementById("chk-agree");
  const nextBtn = document.getElementById("btn-next-license");
  const backBtn = document.getElementById("btn-back-license");

  chk.addEventListener("change", () => {
    licenseAgreed = chk.checked;
    nextBtn.disabled = !licenseAgreed;
  });

  nextBtn.addEventListener("click",  () => { if (licenseAgreed) showStep(2); });
  backBtn.addEventListener("click",  () => showStep(0));
}

function wireOptions() {
  document.getElementById("btn-back-options").addEventListener("click", () => showStep(1));

  document.getElementById("btn-browse").addEventListener("click", async () => {
    const picked = await invoke("browse_dir").catch(() => null);
    if (picked) document.getElementById("install-dir").value = picked;
  });

  document.getElementById("btn-install").addEventListener("click", async () => {
    const opts = {
      install_dir:       document.getElementById("install-dir").value.trim(),
      install_core:      document.getElementById("chk-install-core").checked,
      install_dashboard: document.getElementById("chk-install-dashboard").checked,
      install_cli:       document.getElementById("chk-install-cli").checked,
      start_service:     document.getElementById("chk-service").checked,
      add_to_path:       document.getElementById("chk-path").checked,
      start_menu:        document.getElementById("chk-startmenu").checked,
      desktop_shortcut:  document.getElementById("chk-desktop").checked,
    };
    if (!opts.install_dir) return;

    await showStep(3);
    runInstall(opts);
  });
}

function wireDone() {
  document.getElementById("btn-finish").addEventListener("click", async () => {
    const launch = document.getElementById("chk-launch").checked;
    if (launch) {
      await invoke("launch_dashboard").catch(() => {});
    }
    await win.close();
  });
}

// ── Install runner ───────────────────────────────────────────────────────────

async function runInstall(opts) {
  const fill   = document.getElementById("progress-fill");
  const nextBtn = document.getElementById("btn-next-install");

  // Listen for backend progress events
  const unlisten = await listen("install-progress", (ev) => {
    const { step, pct, message, status } = ev.payload;

    // Update progress bar
    if (fill) fill.style.width = pct + "%";

    // Update matching row
    const row  = document.getElementById(`istep-${step}`);
    const icon = document.getElementById(`istep-icon-${step}`);

    if (row) {
      row.className = `istep ${status}`;
      if (icon) {
        if (status === "running") icon.textContent = "◌";
        else if (status === "ok")   icon.textContent = "✓";
        else if (status === "error") icon.textContent = "✗";
      }
      const lbl = row.querySelector(".istep-label");
      if (lbl && message) lbl.textContent = message;
    }

    if (step === "done" && status === "ok") {
      if (nextBtn) nextBtn.disabled = false;
      nextBtn?.addEventListener("click", () => showStep(4));
    }
  });

  try {
    await invoke("start_install", { options: opts });
  } catch (err) {
    // Mark last running step as error
    const rows = document.querySelectorAll(".istep.running");
    rows.forEach(r => {
      r.className = "istep error";
      const ic = r.querySelector(".istep-icon");
      if (ic) ic.textContent = "✗";
    });
    const stepTitle = document.querySelector(".step-title");
    if (stepTitle) {
      stepTitle.style.background = "none";
      stepTitle.style.webkitTextFillColor = "#f87171";
      stepTitle.style.color = "#f87171";
      stepTitle.textContent = "Installation Failed";
    }
    const sub = document.querySelector(".step-subtitle");
    if (sub) sub.textContent = "Error: " + err;
  } finally {
    unlisten();
  }
}

// ── Utilities ────────────────────────────────────────────────────────────────

function escHtml(s) {
  return String(s)
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
}

// ── Init ─────────────────────────────────────────────────────────────────────

window.addEventListener("DOMContentLoaded", async () => {
  isAdmin = await invoke("check_admin").catch(() => false);
  renderSidebar();
  showStep(0);
});
