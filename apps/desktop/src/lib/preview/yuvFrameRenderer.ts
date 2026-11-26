// Drawing preview frames on a canvas.
//
// Frames arrive as YUV 4:2:0 planes in BT.709 limited range, exactly what the
// export encodes (decision record 0022). WebGL 2 uploads the three planes as
// textures and converts to RGB in a fragment shader, so the CPU never touches the
// pixels. If WebGL 2 is unavailable, the conversion runs in JavaScript instead.

import type { YuvFrame } from "../api/binaryMessages";

/** BT.709 limited range: Y from 16 to 235, U and V from 16 to 240 (8-bit). */
const BLACK_LEVEL = 16 / 255;
const LUMA_SCALE = 255 / 219;
const CHROMA_SCALE = 255 / 224;
const CHROMA_ZERO = 128 / 255;
/** BT.709 YCbCr → RGB coefficients (Kr = 0.2126, Kb = 0.0722). */
const RED_FROM_CR = 1.5748;
const GREEN_FROM_CB = 0.1873;
const GREEN_FROM_CR = 0.4681;
const BLUE_FROM_CB = 1.8556;

const VERTEX_SHADER = `#version 300 es
in vec2 position;
out vec2 textureCoordinate;
void main() {
  textureCoordinate = vec2((position.x + 1.0) / 2.0, (1.0 - position.y) / 2.0);
  gl_Position = vec4(position, 0.0, 1.0);
}`;

const FRAGMENT_SHADER = `#version 300 es
precision mediump float;
in vec2 textureCoordinate;
uniform sampler2D lumaPlane;
uniform sampler2D blueDifferencePlane;
uniform sampler2D redDifferencePlane;
out vec4 color;
void main() {
  float luma = (texture(lumaPlane, textureCoordinate).r - ${BLACK_LEVEL}) * ${LUMA_SCALE};
  float blueDifference = (texture(blueDifferencePlane, textureCoordinate).r - ${CHROMA_ZERO}) * ${CHROMA_SCALE};
  float redDifference = (texture(redDifferencePlane, textureCoordinate).r - ${CHROMA_ZERO}) * ${CHROMA_SCALE};
  color = vec4(
    clamp(luma + ${RED_FROM_CR} * redDifference, 0.0, 1.0),
    clamp(luma - ${GREEN_FROM_CB} * blueDifference - ${GREEN_FROM_CR} * redDifference, 0.0, 1.0),
    clamp(luma + ${BLUE_FROM_CB} * blueDifference, 0.0, 1.0),
    1.0);
}`;

export interface FrameRenderer {
  draw(frame: YuvFrame): void;
  dispose(): void;
}

/** A renderer for `canvas`: WebGL 2 when possible, JavaScript otherwise. */
export function createFrameRenderer(canvas: HTMLCanvasElement): FrameRenderer {
  const gl = canvas.getContext("webgl2", { alpha: false, antialias: false, preserveDrawingBuffer: true });
  if (gl) {
    try {
      return new WebGlFrameRenderer(canvas, gl);
    } catch {
      // Fall through to the JavaScript renderer.
    }
  }
  return new JavaScriptFrameRenderer(canvas);
}

function compileShader(gl: WebGL2RenderingContext, type: number, source: string): WebGLShader {
  const shader = gl.createShader(type);
  if (!shader) {
    throw new Error("could not create a shader");
  }
  gl.shaderSource(shader, source);
  gl.compileShader(shader);
  if (!gl.getShaderParameter(shader, gl.COMPILE_STATUS)) {
    throw new Error(gl.getShaderInfoLog(shader) ?? "shader compilation failed");
  }
  return shader;
}

class WebGlFrameRenderer implements FrameRenderer {
  private readonly textures: WebGLTexture[];

  constructor(
    private readonly canvas: HTMLCanvasElement,
    private readonly gl: WebGL2RenderingContext,
  ) {
    const program = gl.createProgram();
    gl.attachShader(program, compileShader(gl, gl.VERTEX_SHADER, VERTEX_SHADER));
    gl.attachShader(program, compileShader(gl, gl.FRAGMENT_SHADER, FRAGMENT_SHADER));
    gl.linkProgram(program);
    if (!gl.getProgramParameter(program, gl.LINK_STATUS)) {
      throw new Error(gl.getProgramInfoLog(program) ?? "shader linking failed");
    }
    gl.useProgram(program);

    // Two triangles covering the canvas.
    const corners = new Float32Array([-1, -1, 1, -1, -1, 1, -1, 1, 1, -1, 1, 1]);
    gl.bindBuffer(gl.ARRAY_BUFFER, gl.createBuffer());
    gl.bufferData(gl.ARRAY_BUFFER, corners, gl.STATIC_DRAW);
    const positionLocation = gl.getAttribLocation(program, "position");
    gl.enableVertexAttribArray(positionLocation);
    gl.vertexAttribPointer(positionLocation, 2, gl.FLOAT, false, 0, 0);

    gl.pixelStorei(gl.UNPACK_ALIGNMENT, 1);
    this.textures = ["lumaPlane", "blueDifferencePlane", "redDifferencePlane"].map((name, unit) => {
      const texture = gl.createTexture();
      gl.activeTexture(gl.TEXTURE0 + unit);
      gl.bindTexture(gl.TEXTURE_2D, texture);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.LINEAR);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.LINEAR);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
      gl.uniform1i(gl.getUniformLocation(program, name), unit);
      return texture;
    });
  }

  draw(frame: YuvFrame): void {
    const { gl, canvas } = this;
    if (canvas.width !== frame.width || canvas.height !== frame.height) {
      canvas.width = frame.width;
      canvas.height = frame.height;
    }
    const chromaWidth = Math.ceil(frame.width / 2);
    const chromaHeight = Math.ceil(frame.height / 2);
    const planes: Array<[Uint8Array, number, number]> = [
      [frame.yPlane, frame.width, frame.height],
      [frame.uPlane, chromaWidth, chromaHeight],
      [frame.vPlane, chromaWidth, chromaHeight],
    ];
    planes.forEach(([plane, width, height], unit) => {
      gl.activeTexture(gl.TEXTURE0 + unit);
      gl.bindTexture(gl.TEXTURE_2D, this.textures[unit] ?? null);
      gl.texImage2D(gl.TEXTURE_2D, 0, gl.R8, width, height, 0, gl.RED, gl.UNSIGNED_BYTE, plane);
    });
    gl.viewport(0, 0, frame.width, frame.height);
    gl.drawArrays(gl.TRIANGLES, 0, 6);
  }

  dispose(): void {
    for (const texture of this.textures) {
      this.gl.deleteTexture(texture);
    }
  }
}

class JavaScriptFrameRenderer implements FrameRenderer {
  constructor(private readonly canvas: HTMLCanvasElement) {}

  draw(frame: YuvFrame): void {
    const context = this.canvas.getContext("2d");
    if (!context) {
      return;
    }
    this.canvas.width = frame.width;
    this.canvas.height = frame.height;
    const image = context.createImageData(frame.width, frame.height);
    const chromaWidth = Math.ceil(frame.width / 2);
    for (let row = 0; row < frame.height; row += 1) {
      for (let column = 0; column < frame.width; column += 1) {
        const chromaIndex = (row >> 1) * chromaWidth + (column >> 1);
        const luma = ((frame.yPlane[row * frame.width + column] ?? 0) / 255 - BLACK_LEVEL) * LUMA_SCALE;
        const blueDifference = ((frame.uPlane[chromaIndex] ?? 128) / 255 - CHROMA_ZERO) * CHROMA_SCALE;
        const redDifference = ((frame.vPlane[chromaIndex] ?? 128) / 255 - CHROMA_ZERO) * CHROMA_SCALE;
        const pixel = (row * frame.width + column) * 4;
        image.data[pixel] = (luma + RED_FROM_CR * redDifference) * 255;
        image.data[pixel + 1] = (luma - GREEN_FROM_CB * blueDifference - GREEN_FROM_CR * redDifference) * 255;
        image.data[pixel + 2] = (luma + BLUE_FROM_CB * blueDifference) * 255;
        image.data[pixel + 3] = 255;
      }
    }
    context.putImageData(image, 0, 0);
  }

  dispose(): void {}
}
