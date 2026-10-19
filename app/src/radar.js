// Radar overlay (first: a simulated scanner, to test the stack). Spokes arrive from
// Rust as raw bytes over a Tauri channel and are written straight into a texture
// (one row per spoke); a MapLibre custom WebGL layer draws the texture as a disc
// round the boat, the fragment shader turning each pixel into (range, bearing),
// so the GPU does all the polar-to-chart work.
import * as maplibregl from "maplibre-gl";
import { invoke, Channel } from "@tauri-apps/api/core";

const VS = `#version 300 es
uniform mat4 u_matrix;
uniform vec2 u_centre;   // mercator 0..1
uniform float u_radius;  // mercator units
in vec2 a_pos;           // -1..1
out vec2 v_pos;
void main() {
  v_pos = a_pos;
  gl_Position = u_matrix * vec4(u_centre + a_pos * u_radius, 0.0, 1.0);
}`;
const FS = `#version 300 es
precision highp float;
uniform sampler2D u_tex;
uniform float u_sweep;   // the spoke just written, 0..1 of a turn
uniform float u_opacity;
in vec2 v_pos;
out vec4 frag;
void main() {
  float r = length(v_pos);
  if (r > 1.0) discard;
  // bearing from north, clockwise (mercator y grows southwards)
  float b = atan(v_pos.x, -v_pos.y) / 6.28318531;
  if (b < 0.0) b += 1.0;
  float v = texture(u_tex, vec2(r, b)).r;
  // the age of this spoke since the sweep passed: a slight afterglow fade
  float age = fract(u_sweep - b);
  float a = smoothstep(0.12, 0.30, v) * (1.0 - 0.35 * age);
  vec3 c = mix(vec3(0.15, 0.85, 0.25), vec3(1.0, 0.85, 0.1), smoothstep(0.3, 0.6, v));
  c = mix(c, vec3(0.95, 0.15, 0.1), smoothstep(0.6, 0.9, v));
  // range ring at the edge and a faint sweep line
  float ring = smoothstep(0.985, 0.995, r) * 0.6;
  float sweep = (1.0 - smoothstep(0.0, 0.004, abs(fract(b - u_sweep + 0.5) - 0.5))) * 0.5;
  frag = vec4(c * a + vec3(ring + sweep), max(a * u_opacity, max(ring, sweep)));
}`;

export function setupRadar(map, { getBoat }) {
  const $ = (id) => document.getElementById(id);
  let gl = null, prog = null, buf = null, tex = null;
  let spokes = 2048, samples = 1024, sweep = 0, rangeNm = 3, centre = null, running = false;
  // what the page keeps up with: frames drawn, packets, bytes, time spent writing the texture
  const st = { frames: 0, packets: 0, bytes: 0, uploadMs: 0, maxGapMs: 0, lastPacket: 0, t0: 0 };

  const layer = {
    id: "radar",
    type: "custom",
    renderingMode: "2d",
    onAdd(_m, g) {
      gl = g;
      const sh = (type, src) => { const s = gl.createShader(type); gl.shaderSource(s, src); gl.compileShader(s); if (!gl.getShaderParameter(s, gl.COMPILE_STATUS)) console.error(gl.getShaderInfoLog(s)); return s; };
      prog = gl.createProgram();
      gl.attachShader(prog, sh(gl.VERTEX_SHADER, VS));
      gl.attachShader(prog, sh(gl.FRAGMENT_SHADER, FS));
      gl.linkProgram(prog);
      buf = gl.createBuffer();
      gl.bindBuffer(gl.ARRAY_BUFFER, buf);
      gl.bufferData(gl.ARRAY_BUFFER, new Float32Array([-1, -1, 1, -1, -1, 1, 1, 1]), gl.STATIC_DRAW);
      makeTexture();
    },
    render(g, args) {
      if (!running || !centre) return;
      st.frames++;
      const m = maplibregl.MercatorCoordinate.fromLngLat(centre);
      gl.useProgram(prog);
      gl.uniformMatrix4fv(gl.getUniformLocation(prog, "u_matrix"), false, args.defaultProjectionData.mainMatrix);
      gl.uniform2f(gl.getUniformLocation(prog, "u_centre"), m.x, m.y);
      gl.uniform1f(gl.getUniformLocation(prog, "u_radius"), rangeNm * 1852 * m.meterInMercatorCoordinateUnits());
      gl.uniform1f(gl.getUniformLocation(prog, "u_sweep"), sweep);
      gl.uniform1f(gl.getUniformLocation(prog, "u_opacity"), 0.85);
      gl.activeTexture(gl.TEXTURE0);
      gl.bindTexture(gl.TEXTURE_2D, tex);
      gl.uniform1i(gl.getUniformLocation(prog, "u_tex"), 0);
      gl.bindBuffer(gl.ARRAY_BUFFER, buf);
      const loc = gl.getAttribLocation(prog, "a_pos");
      gl.enableVertexAttribArray(loc);
      gl.vertexAttribPointer(loc, 2, gl.FLOAT, false, 0, 0);
      gl.enable(gl.BLEND);
      gl.blendFunc(gl.ONE, gl.ONE_MINUS_SRC_ALPHA);
      gl.drawArrays(gl.TRIANGLE_STRIP, 0, 4);
    },
  };
  function makeTexture() {
    if (!gl) return;
    if (tex) gl.deleteTexture(tex);
    tex = gl.createTexture();
    gl.bindTexture(gl.TEXTURE_2D, tex);
    gl.pixelStorei(gl.UNPACK_ALIGNMENT, 1);
    gl.texImage2D(gl.TEXTURE_2D, 0, gl.R8, samples, spokes, 0, gl.RED, gl.UNSIGNED_BYTE, new Uint8Array(samples * spokes));
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.LINEAR);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.LINEAR);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.REPEAT);
  }

  /** A packet from Rust: header then spoke rows, written into the texture. */
  function onPacket(msg) {
    const ab = msg instanceof ArrayBuffer ? msg : msg?.buffer instanceof ArrayBuffer ? msg.buffer : Array.isArray(msg) ? new Uint8Array(msg).buffer : null;
    if (!ab || !gl) return;
    const now = performance.now();
    if (st.lastPacket) st.maxGapMs = Math.max(st.maxGapMs, now - st.lastPacket);
    st.lastPacket = now;
    const dv = new DataView(ab);
    const first = dv.getUint32(0, true), count = dv.getUint16(4, true), n = dv.getUint16(6, true), turn = dv.getUint16(8, true);
    if (n !== samples || turn !== spokes) { samples = n; spokes = turn; makeTexture(); }
    const data = new Uint8Array(ab, 12, count * n);
    const t = performance.now();
    gl.bindTexture(gl.TEXTURE_2D, tex);
    gl.pixelStorei(gl.UNPACK_ALIGNMENT, 1);
    const a = Math.min(count, spokes - first);
    gl.texSubImage2D(gl.TEXTURE_2D, 0, 0, first, n, a, gl.RED, gl.UNSIGNED_BYTE, data.subarray(0, a * n));
    if (a < count) gl.texSubImage2D(gl.TEXTURE_2D, 0, 0, 0, n, count - a, gl.RED, gl.UNSIGNED_BYTE, data.subarray(a * n));
    st.uploadMs += performance.now() - t;
    st.packets++;
    st.bytes += ab.byteLength;
    sweep = ((first + count) % spokes) / spokes;
    map.triggerRepaint();
  }

  async function start() {
    const [sp, sa] = $("rs-res").value.split("x").map(Number);
    spokes = sp; samples = sa;
    rangeNm = Number($("rs-range").value) || 3;
    const b = getBoat();
    centre = b ? [b[0], b[1]] : [map.getCenter().lng, map.getCenter().lat];
    if (!map.getLayer("radar")) map.addLayer(layer, "boat");
    makeTexture();
    Object.assign(st, { frames: 0, packets: 0, bytes: 0, uploadMs: 0, maxGapMs: 0, lastPacket: 0, t0: performance.now() });
    const ch = new Channel();
    ch.onmessage = onPacket;
    running = true;
    await invoke("radar_sim_start", { rpm: Number($("rs-rpm").value), spokes, samples, channel: ch });
    $("rs-go").textContent = "Stop";
  }
  function stop() {
    running = false;
    invoke("radar_sim_stop").catch(() => {});
    $("rs-go").textContent = "Start";
    map.triggerRepaint();
  }
  $("rs-go").onclick = () => (running ? stop() : start().catch((e) => { $("rs-stats").textContent = String(e); stop(); }));
  $("rs-range").onchange = () => { rangeNm = Number($("rs-range").value) || 3; map.triggerRepaint(); };

  // the numbers, every second: rates over the last second, totals since the start
  let prev = null;
  setInterval(async () => {
    if (!running) return;
    const r = await invoke("radar_sim_stats").catch(() => null);
    const now = performance.now();
    const cur = { t: now, frames: st.frames, packets: st.packets, bytes: st.bytes, upload: st.uploadMs, spokes: r?.spokes ?? 0, sent: r?.bytes ?? 0, gen: r?.gen_ms ?? 0 };
    if (prev) {
      const s = (now - prev.t) / 1000;
      const fps = (cur.frames - prev.frames) / s;
      const mbs = (cur.bytes - prev.bytes) / s / 1e6;
      const backlog = cur.sent - cur.bytes; // sent by Rust but not yet arrived here
      const mem = performance.memory ? ` · JS heap ${(performance.memory.usedJSHeapSize / 1e6).toFixed(0)} MB` : "";
      $("rs-stats").textContent =
        `Chart ${fps.toFixed(0)} frames/s · ${((cur.packets - prev.packets) / s).toFixed(0)} packets/s · ${mbs.toFixed(2)} MB/s\n` +
        `Spokes ${((cur.spokes - prev.spokes) / s).toFixed(0)}/s · texture writes ${((cur.upload - prev.upload) / s).toFixed(1)} ms/s · Rust echo making ${((cur.gen - prev.gen) / s).toFixed(0)} ms/s\n` +
        `Backlog ${(Math.max(0, backlog) / 1e3).toFixed(0)} kB · longest gap between packets ${st.maxGapMs.toFixed(0)} ms${mem}`;
      st.maxGapMs = 0;
    }
    prev = cur;
  }, 1000);

  return { running: () => running };
}
