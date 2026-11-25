// Page controls. The AudioContext starts on the first Play (browsers require a user gesture) and is suspended
// shortly after every pause, so a paused page does no audio work.
const KINDS = ["White", "Pink", "Brown", "Blue", "Violet"];
const ACCENTS = ["#8c96a8", "#d4708f", "#d08f4a", "#4f8fd6", "#9b74d6"];
const FADE_MS = 150;

const state = { kind: 2, vol: 50, playing: false, ...load() };
let wasm = loadWasm();
let audio = null;
let suspendTimer = 0;

const $ = (id) => document.getElementById(id);

// Storage can be blocked (private modes, site-data settings) or hold junk; the page then just forgets.
function load() {
  try {
    const { kind, vol } = JSON.parse(localStorage.getItem("noiseman") ?? "{}");
    const valid = (v, max) => Number.isInteger(v) && v >= 0 && v <= max;
    return { ...(valid(kind, KINDS.length - 1) && { kind }), ...(valid(vol, 100) && { vol }) };
  } catch {
    return {};
  }
}

function loadWasm() {
  return fetch("noiseman.wasm")
    .then((r) => (r.ok ? r.arrayBuffer() : Promise.reject(new Error(`noiseman.wasm: HTTP ${r.status}`))))
    .then((bytes) => WebAssembly.compile(bytes));
}
const buttons = KINDS.map((label, i) => {
  const b = document.createElement("button");
  b.type = "button";
  b.setAttribute("role", "radio");
  b.innerHTML = `<span>${i + 1}</span>${label}`;
  b.addEventListener("click", () => setKind(i));
  $("kinds").append(b);
  return b;
});

async function startAudio() {
  const ctx = new AudioContext({ latencyHint: "playback" });
  try {
    wasm ??= loadWasm();
    const [module] = await Promise.all([wasm, ctx.audioWorklet.addModule("worklet.js")]);
    const seed = crypto.getRandomValues(new Uint32Array(1))[0];
    const node = new AudioWorkletNode(ctx, "noiseman", {
      numberOfInputs: 0,
      outputChannelCount: [1],
      processorOptions: { module, seed, kind: state.kind },
    });
    node.connect(ctx.destination);
    return { ctx, node };
  } catch (e) {
    ctx.close();
    wasm = null;
    throw e;
  }
}

function send() {
  audio?.then(({ node }) => node.port.postMessage({ kind: state.kind, vol: state.playing ? state.vol : 0 }));
}

async function setPlaying(on) {
  state.playing = on;
  render();
  clearTimeout(suspendTimer);
  try {
    audio ??= startAudio();
    const { ctx } = await audio;
    if (state.playing !== on) return;
    if (on) await ctx.resume();
    send();
    if (!on) suspendTimer = setTimeout(() => ctx.suspend(), FADE_MS);
  } catch (e) {
    audio = null;
    state.playing = false;
    render(`Cannot start audio: ${e.message}`);
  }
}

function setKind(i) {
  state.kind = (i + KINDS.length) % KINDS.length;
  save();
}

function setVol(v) {
  state.vol = Math.min(100, Math.max(0, v));
  save();
}

function save() {
  try {
    localStorage.setItem("noiseman", JSON.stringify({ kind: state.kind, vol: state.vol }));
  } catch {
    // Not remembered across visits; playback is unaffected.
  }
  render();
  send();
}

function render(error) {
  document.documentElement.style.setProperty("--accent", ACCENTS[state.kind]);
  document.body.classList.toggle("playing", state.playing);
  buttons.forEach((b, i) => {
    b.setAttribute("aria-checked", i === state.kind);
    b.tabIndex = i === state.kind ? 0 : -1;
  });
  if (buttons.includes(document.activeElement)) buttons[state.kind].focus();
  $("vol").value = state.vol;
  $("volout").value = `${state.vol}%`;
  $("status").textContent = state.playing ? "● Playing" : "◌ Paused";
  $("play").textContent = state.playing ? "Pause" : "Play";
  $("error").hidden = !error;
  $("error").textContent = error ?? "";
}

$("vol").addEventListener("input", (e) => setVol(+e.target.value));
$("play").addEventListener("click", () => setPlaying(!state.playing));
addEventListener("keydown", (e) => {
  if (e.ctrlKey || e.metaKey || e.altKey) return;
  const inRange = e.target === $("vol");
  const onPlay = e.target === $("play");
  if (e.key >= "1" && e.key <= "5") setKind(+e.key - 1);
  else if (e.key === "k" || (e.key === "ArrowUp" && !inRange)) setKind(state.kind - 1);
  else if (e.key === "j" || (e.key === "ArrowDown" && !inRange)) setKind(state.kind + 1);
  else if (e.key === "h" || (e.key === "ArrowLeft" && !inRange)) setVol(state.vol - 5);
  else if (e.key === "l" || (e.key === "ArrowRight" && !inRange)) setVol(state.vol + 5);
  else if (e.key === "m" || (e.key === " " && !onPlay)) {
    if (!e.repeat) setPlaying(!state.playing);
  }
  else return;
  e.preventDefault();
});

render();
