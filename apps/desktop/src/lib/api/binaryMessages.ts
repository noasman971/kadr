// Decoders for the binary answers of the back end. The layouts are written in
// `src-tauri/src/binary_messages.rs`; every number is little-endian.

/** Bytes of the width and height header of pictures. */
const SIZE_HEADER_LENGTH = 8;

/** A preview frame: YUV 4:2:0 planes in BT.709 limited range. */
export interface YuvFrame {
  width: number;
  height: number;
  yPlane: Uint8Array;
  uPlane: Uint8Array;
  vPlane: Uint8Array;
}

/** A thumbnail: RGBA pixels, straight alpha. */
export interface RgbaPicture {
  width: number;
  height: number;
  pixels: Uint8ClampedArray<ArrayBuffer>;
}

/** A waveform: one loudness byte (0–255) per slice of `1 / peaksPerSecond` s. */
export interface Waveform {
  peaksPerSecond: number;
  peaks: Uint8Array;
}

function readSize(buffer: ArrayBuffer): { width: number; height: number } {
  const header = new DataView(buffer, 0, SIZE_HEADER_LENGTH);
  return { width: header.getUint32(0, true), height: header.getUint32(4, true) };
}

/** Width of the colour planes of a picture `width` wide. */
export function chromaLength(side: number): number {
  return Math.ceil(side / 2);
}

export function decodeYuvFrame(buffer: ArrayBuffer): YuvFrame {
  const { width, height } = readSize(buffer);
  const lumaLength = width * height;
  const chromaPlaneLength = chromaLength(width) * chromaLength(height);
  const expectedLength = SIZE_HEADER_LENGTH + lumaLength + 2 * chromaPlaneLength;
  if (buffer.byteLength !== expectedLength) {
    throw new Error(`a ${width}×${height} frame needs ${expectedLength} bytes, got ${buffer.byteLength}`);
  }
  let offset = SIZE_HEADER_LENGTH;
  const yPlane = new Uint8Array(buffer, offset, lumaLength);
  offset += lumaLength;
  const uPlane = new Uint8Array(buffer, offset, chromaPlaneLength);
  offset += chromaPlaneLength;
  const vPlane = new Uint8Array(buffer, offset, chromaPlaneLength);
  return { width, height, yPlane, uPlane, vPlane };
}

export function decodeRgbaPicture(buffer: ArrayBuffer): RgbaPicture {
  const { width, height } = readSize(buffer);
  const pixels = new Uint8ClampedArray(buffer, SIZE_HEADER_LENGTH, width * height * 4);
  return { width, height, pixels };
}

export function decodeWaveform(buffer: ArrayBuffer): Waveform {
  const peaksPerSecond = new DataView(buffer, 0, 4).getUint32(0, true);
  return { peaksPerSecond, peaks: new Uint8Array(buffer, 4) };
}

/** Interleaved stereo samples. */
export function decodeSound(buffer: ArrayBuffer): Float32Array {
  // The bytes come straight from the IPC body, aligned for 32-bit reads.
  return new Float32Array(buffer);
}
