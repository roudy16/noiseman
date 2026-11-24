// Runs on the audio thread: one noiseman-core synth (noiseman.wasm) rendering mono blocks. No allocation per block.
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
    const ptr = this.wasm.synth_render(this.kind, this.vol, out.length);
    const view = this.view;
    if (!view || view.byteOffset !== ptr || view.length !== out.length || view.buffer !== this.wasm.memory.buffer) {
      this.view = new Float32Array(this.wasm.memory.buffer, ptr, out.length);
    }
    out.set(this.view);
    return true;
  }
}

registerProcessor("noiseman", Noiseman);
