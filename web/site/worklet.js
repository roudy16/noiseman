// Runs on the audio thread: one noiseman-core synth (noiseman.wasm) rendering mono blocks. No allocation per block.
// synth_render fills at most MAX_FRAMES (web/src/lib.rs), so a larger render quantum is rendered in chunks.
const MAX_FRAMES = 1024;

class Noiseman extends AudioWorkletProcessor {
  constructor({ processorOptions: { module, seed, kind } }) {
    super();
    this.wasm = new WebAssembly.Instance(module).exports;
    this.wasm.synth_init(sampleRate, seed, kind);
    this.kind = kind;
    this.vol = 0;
    this.view = null;
    this.port.onmessage = ({ data }) => ({ kind: this.kind, vol: this.vol } = data);
  }

  process(_inputs, [[out]]) {
    for (let at = 0; at < out.length; at += MAX_FRAMES) {
      const n = Math.min(MAX_FRAMES, out.length - at);
      const ptr = this.wasm.synth_render(this.kind, this.vol, n);
      const view = this.view;
      if (!view || view.byteOffset !== ptr || view.length !== n || view.buffer !== this.wasm.memory.buffer) {
        this.view = new Float32Array(this.wasm.memory.buffer, ptr, n);
      }
      out.set(this.view, at);
    }
    return true;
  }
}

registerProcessor("noiseman", Noiseman);
