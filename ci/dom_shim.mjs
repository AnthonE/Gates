// A four-line DOM for node, imported for its side effect ONLY.
//
// `ez-tree` builds a `THREE.TextureLoader` at module scope and calls `.load()`,
// which reaches for `Image` and `document`. The geometry it generates needs
// neither — but ESM hoists imports and runs them in order, so the shim cannot
// live at the top of the file that needs it. It has to be its own module,
// imported first.
//
// This is what lets `ci/pine_shape.mjs` score the SHIPPED tree builder instead
// of a copy of it, which is the whole reason `web/src/props.js` is a module
// with no DOM of its own.
globalThis.document = {
  createElementNS: () => ({ style: {}, setAttribute() {}, addEventListener() {} }),
  createElement: () => ({ style: {}, getContext: () => null, setAttribute() {}, addEventListener() {} }),
};
globalThis.Image = class {
  constructor() {
    this.style = {};
  }
  addEventListener() {}
  removeEventListener() {}
  set src(_v) {}
};
