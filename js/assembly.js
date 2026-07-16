import * as THREE from 'three';
import { GLTFLoader } from 'three/addons/loaders/GLTFLoader.js';
import { CHAPTERS, chapterAt, chapterLocal, syncOverlay } from './story.js?v=104';

/**
 * Pinned workbench: sci-fi table + silicone mat.
 * Parts start scattered on the table, then assemble onto the mat.
 */

const CONFIG = {
  scrollVh: 13.5,
  modelPath: 'assets/models/OASIS_ASM.gltf?v=22',
  chapters: CHAPTERS,
  pressDepth: 0.0028,
  // Silicone repair mat sits on the sci-fi table; assembly lands on the mat.
  // Biased right so the five scatter seats clear the story card (~left 28%).
  mat: {
    w: 0.24,
    d: 0.17,
    h: 0.0042,
    x: 0.1,
    z: 0.02,
  },
  deskSlots: {
    // Five seats clear of the mat (camera from +z): NW / N / NE / SW / E
    case: { x: -0.04, z: -0.135 },
    pcb: { x: 0.1, z: 0.02, onMat: true },
    buttons: [
      { x: 0.12, z: -0.145 },
      { x: -0.1, z: 0.145 },
      { x: 0.3, z: 0.05 },
    ],
    top: { x: 0.3, z: -0.12 },
  },
  desk: {
    case: { pos: [0, 0, 0], rot: [Math.PI / 2, 0.22, 0.04] },
    pcb: { pos: [0, 0, 0], rot: [Math.PI / 2, -0.12, 0.02] },
    buttons: [
      { pos: [0, 0, 0], rot: [0.35, 0.45, 0.08] },
      { pos: [0, 0, 0], rot: [0.2, -0.55, 0.15] },
      { pos: [0, 0, 0], rot: [0.4, 0.15, -0.2] },
    ],
    top: { pos: [0, 0, 0], rot: [Math.PI / 2, -0.2, 0.05] },
  },
  // Clear approach poses so the final settle is a short slide, never through the board
  approach: {
    case: { pos: [0, 0.014, -0.07], rot: [0, 0, 0] },
    top: { pos: [0, 0.016, 0.08], rot: [0, 0, 0] },
    buttons: { pos: [0, 0.01, 0.07], rot: [0, 0, 0] },
  },
  // High staging spots well in front of the pad
  park: {
    case: { pos: [-0.07, 0.08, 0.04], rot: [0.3, 0.4, 0.05] },
    top: { pos: [0.07, 0.082, 0.11], rot: [0.35, -0.35, 0] },
    buttons: [
      { pos: [-0.036, 0.075, 0.12], rot: [0.3, 0.2, 0] },
      { pos: [0.0, 0.078, 0.125], rot: [0.25, 0, 0] },
      { pos: [0.036, 0.075, 0.12], rot: [0.3, -0.2, 0] },
    ],
  },
  materials: {
    case: { color: 0xf6f6f4, roughness: 0.55, metalness: 0.04 },
    pcb: { color: 0x4a6741, roughness: 0.75, metalness: 0.1 },
    button: { color: 0xfafaf8, roughness: 0.45, metalness: 0.06 },
    top: { color: 0xf6f6f4, roughness: 0.55, metalness: 0.04 },
    desk: { color: 0xeef1f4, roughness: 0.88, metalness: 0.04 },
  },
};

function chapterById(id) {
  return CONFIG.chapters.find((c) => c.id === id);
}

const canvas = document.getElementById('assembly-canvas');

if (!canvas) throw new Error('Assembly canvas not found');

const scene = new THREE.Scene();
scene.background = null;

const camera = new THREE.PerspectiveCamera(36, 1, 0.001, 20);
camera.position.set(0.08, 0.06, 0.22);

const renderer = new THREE.WebGLRenderer({ canvas, antialias: true, alpha: true, powerPreference: 'high-performance' });
renderer.setClearColor(0x000000, 0);
renderer.setPixelRatio(Math.min(window.devicePixelRatio, 1.75));
renderer.shadowMap.enabled = true;
renderer.shadowMap.type = THREE.PCFSoftShadowMap;
renderer.outputColorSpace = THREE.SRGBColorSpace;
renderer.toneMapping = THREE.ACESFilmicToneMapping;
renderer.toneMappingExposure = 1.02;

const assembly = new THREE.Group();
assembly.scale.setScalar(1.22);
scene.add(assembly);

// Soft studio lighting — matches light app theme, no blown highlights
scene.add(new THREE.AmbientLight(0xffffff, 0.55));

const hemi = new THREE.HemisphereLight(0xf5f8fa, 0xc8d2dc, 0.35);
scene.add(hemi);

const keyLight = new THREE.DirectionalLight(0xfff8f2, 0.82);
keyLight.position.set(1.2, 3.2, 1.8);
keyLight.castShadow = true;
keyLight.shadow.mapSize.set(1024, 1024);
keyLight.shadow.camera.near = 0.05;
keyLight.shadow.camera.far = 6;
keyLight.shadow.camera.left = -0.4;
keyLight.shadow.camera.right = 0.4;
keyLight.shadow.camera.top = 0.4;
keyLight.shadow.camera.bottom = -0.4;
keyLight.shadow.bias = -0.00012;
keyLight.shadow.normalBias = 0.002;
keyLight.shadow.radius = 4.5;
scene.add(keyLight);
keyLight.target.position.set(0, -0.02, 0.03);
scene.add(keyLight.target);

const fillLight = new THREE.DirectionalLight(0xd8e4ee, 0.38);
fillLight.position.set(-2.2, 1.6, -0.6);
scene.add(fillLight);

const rimLight = new THREE.DirectionalLight(0xb0c4d4, 0.22);
rimLight.position.set(-0.4, 1.0, -2.2);
scene.add(rimLight);

/** Light grey workbench — soft blueprint grid, app-theme friendly. */
function makeDeskMaps(size = 1024) {
  const colorCanvas = document.createElement('canvas');
  colorCanvas.width = colorCanvas.height = size;
  const ctx = colorCanvas.getContext('2d');

  const roughCanvas = document.createElement('canvas');
  roughCanvas.width = roughCanvas.height = size;
  const rctx = roughCanvas.getContext('2d');

  // Soft off-white / cool grey base (matches --bg / cards)
  ctx.fillStyle = '#EEF1F4';
  ctx.fillRect(0, 0, size, size);

  // Gentle center wash
  const glow = ctx.createRadialGradient(size * 0.5, size * 0.5, size * 0.08, size * 0.5, size * 0.5, size * 0.55);
  glow.addColorStop(0, 'rgba(255,255,255,0.45)');
  glow.addColorStop(0.55, 'rgba(232,238,242,0.2)');
  glow.addColorStop(1, 'rgba(200,210,220,0.15)');
  ctx.fillStyle = glow;
  ctx.fillRect(0, 0, size, size);

  // Large panel tiles — blueprint soft
  const tile = size / 4;
  ctx.strokeStyle = 'rgba(74, 144, 196, 0.14)';
  ctx.lineWidth = 1.5;
  for (let x = 0; x <= size; x += tile) {
    ctx.beginPath();
    ctx.moveTo(x + 0.5, 0);
    ctx.lineTo(x + 0.5, size);
    ctx.stroke();
  }
  for (let y = 0; y <= size; y += tile) {
    ctx.beginPath();
    ctx.moveTo(0, y + 0.5);
    ctx.lineTo(size, y + 0.5);
    ctx.stroke();
  }

  // Fine grid
  const fine = size / 32;
  ctx.strokeStyle = 'rgba(74, 144, 196, 0.07)';
  ctx.lineWidth = 1;
  for (let x = 0; x <= size; x += fine) {
    ctx.beginPath();
    ctx.moveTo(x + 0.5, 0);
    ctx.lineTo(x + 0.5, size);
    ctx.stroke();
  }
  for (let y = 0; y <= size; y += fine) {
    ctx.beginPath();
    ctx.moveTo(0, y + 0.5);
    ctx.lineTo(size, y + 0.5);
    ctx.stroke();
  }

  // Quiet corner marks
  const mark = (x, y, flipX, flipY) => {
    ctx.strokeStyle = 'rgba(74, 144, 196, 0.28)';
    ctx.lineWidth = 1.5;
    ctx.beginPath();
    ctx.moveTo(x, y + flipY * 22);
    ctx.lineTo(x, y);
    ctx.lineTo(x + flipX * 22, y);
    ctx.stroke();
  };
  const inset = size * 0.08;
  mark(inset, inset, 1, 1);
  mark(size - inset, inset, -1, 1);
  mark(inset, size - inset, 1, -1);
  mark(size - inset, size - inset, -1, -1);

  // Soft edge falloff into blueprint page
  const vig = ctx.createRadialGradient(size * 0.5, size * 0.5, size * 0.28, size * 0.5, size * 0.5, size * 0.72);
  vig.addColorStop(0, 'rgba(0,0,0,0)');
  vig.addColorStop(1, 'rgba(180,190,200,0.22)');
  ctx.fillStyle = vig;
  ctx.fillRect(0, 0, size, size);

  // Micro noise
  const img = ctx.getImageData(0, 0, size, size);
  const d = img.data;
  for (let i = 0; i < d.length; i += 20) {
    const n = (Math.random() - 0.5) * 6;
    d[i] = Math.max(0, Math.min(255, d[i] + n));
    d[i + 1] = Math.max(0, Math.min(255, d[i + 1] + n));
    d[i + 2] = Math.max(0, Math.min(255, d[i + 2] + n));
  }
  ctx.putImageData(img, 0, 0);

  // Roughness: mostly matte laminate
  rctx.fillStyle = '#c8c8c8';
  rctx.fillRect(0, 0, size, size);
  rctx.strokeStyle = '#d8d8d8';
  rctx.lineWidth = 2;
  for (let x = 0; x <= size; x += tile) {
    rctx.beginPath();
    rctx.moveTo(x, 0);
    rctx.lineTo(x, size);
    rctx.stroke();
  }
  for (let y = 0; y <= size; y += tile) {
    rctx.beginPath();
    rctx.moveTo(0, y);
    rctx.lineTo(size, y);
    rctx.stroke();
  }

  const map = new THREE.CanvasTexture(colorCanvas);
  map.colorSpace = THREE.SRGBColorSpace;
  map.anisotropy = Math.min(8, renderer.capabilities.getMaxAnisotropy());
  map.wrapS = map.wrapT = THREE.ClampToEdgeWrapping;

  const roughnessMap = new THREE.CanvasTexture(roughCanvas);
  roughnessMap.wrapS = roughnessMap.wrapT = THREE.ClampToEdgeWrapping;
  roughnessMap.anisotropy = map.anisotropy;

  return { map, roughnessMap };
}

const deskMaps = makeDeskMaps(1024);

const desk = new THREE.Mesh(
  new THREE.PlaneGeometry(0.75, 0.55, 1, 1),
  new THREE.MeshStandardMaterial({
    color: 0xffffff,
    map: deskMaps.map,
    roughnessMap: deskMaps.roughnessMap,
    roughness: 0.88,
    metalness: 0.04,
  }),
);
desk.rotation.x = -Math.PI / 2;
desk.position.y = -0.038;
desk.receiveShadow = true;
scene.add(desk);

// Soft grey rim
const deskRim = new THREE.Mesh(
  new THREE.PlaneGeometry(0.78, 0.58),
  new THREE.MeshStandardMaterial({
    color: 0xd5dde4,
    roughness: 0.95,
    metalness: 0.02,
  }),
);
deskRim.rotation.x = -Math.PI / 2;
deskRim.position.y = -0.0395;
deskRim.receiveShadow = false;
scene.add(deskRim);

const deskShadow = new THREE.Mesh(
  new THREE.PlaneGeometry(0.75, 0.55),
  new THREE.ShadowMaterial({ opacity: 0.2, color: 0x4a5a68 }),
);
deskShadow.rotation.x = -Math.PI / 2;
deskShadow.position.y = -0.0368;
deskShadow.receiveShadow = true;
scene.add(deskShadow);

/** Cyan silicone repair mat — sits on the table; device assembles on it. */
function makeSiliconeMatMap(size = 512) {
  const canvas = document.createElement('canvas');
  canvas.width = canvas.height = size;
  const ctx = canvas.getContext('2d');

  // Full blue silicone body
  ctx.fillStyle = '#1f8fc4';
  ctx.fillRect(0, 0, size, size);

  // Soft rubber variation
  for (let i = 0; i < 1800; i++) {
    const x = Math.random() * size;
    const y = Math.random() * size;
    const a = 0.02 + Math.random() * 0.05;
    ctx.fillStyle = Math.random() > 0.5
      ? `rgba(120,200,230,${a})`
      : `rgba(10,50,80,${a})`;
    ctx.fillRect(x, y, 1 + Math.random() * 2, 1);
  }

  const inset = size * 0.06;
  const trayH = size * 0.18;
  const gap = size * 0.025;
  const trayY = inset;
  const trayColors = '#1677a8';

  // Top tray pockets (4 small + 1 wide)
  const smallW = (size - inset * 2 - gap * 4) / 5.4;
  let x = inset;
  for (let i = 0; i < 4; i++) {
    ctx.fillStyle = trayColors;
    roundRect(ctx, x, trayY, smallW, trayH, size * 0.012);
    ctx.fill();
    x += smallW + gap;
  }
  const wideW = size - inset - x;
  ctx.fillStyle = trayColors;
  roundRect(ctx, x, trayY, wideW, trayH, size * 0.012);
  ctx.fill();

  // Main work recess
  const workY = trayY + trayH + gap * 1.2;
  ctx.fillStyle = '#1a86b8';
  roundRect(ctx, inset, workY, size - inset * 2, size - workY - inset, size * 0.018);
  ctx.fill();

  // Divider ridge highlight
  ctx.fillStyle = 'rgba(80, 180, 220, 0.35)';
  ctx.fillRect(inset, workY - gap * 0.35, size - inset * 2, gap * 0.45);

  const map = new THREE.CanvasTexture(canvas);
  map.colorSpace = THREE.SRGBColorSpace;
  map.anisotropy = Math.min(8, renderer.capabilities.getMaxAnisotropy());
  return map;
}

function roundRect(ctx, x, y, w, h, r) {
  const rr = Math.min(r, w * 0.5, h * 0.5);
  ctx.beginPath();
  ctx.moveTo(x + rr, y);
  ctx.arcTo(x + w, y, x + w, y + h, rr);
  ctx.arcTo(x + w, y + h, x, y + h, rr);
  ctx.arcTo(x, y + h, x, y, rr);
  ctx.arcTo(x, y, x + w, y, rr);
  ctx.closePath();
}

function createSiliconeMat() {
  const { w: W, d: D, h: H } = CONFIG.mat;
  const group = new THREE.Group();
  group.name = 'SiliconeMat';

  const blue = 0x1f8fc4;
  const silicone = new THREE.MeshStandardMaterial({
    color: blue,
    roughness: 0.98,
    metalness: 0,
  });

  const base = new THREE.Mesh(new THREE.BoxGeometry(W, H, D), silicone);
  base.castShadow = true;
  base.receiveShadow = true;
  group.add(base);

  const map = makeSiliconeMatMap(768);
  const top = new THREE.Mesh(
    new THREE.PlaneGeometry(W * 0.995, D * 0.995),
    new THREE.MeshStandardMaterial({
      map,
      color: 0x8ec8e8,
      roughness: 0.98,
      metalness: 0,
    }),
  );
  top.rotation.x = -Math.PI / 2;
  top.position.y = H * 0.5 + 0.00025;
  top.receiveShadow = true;
  group.add(top);

  const lipMat = new THREE.MeshStandardMaterial({
    color: 0x1677a8,
    roughness: 0.97,
    metalness: 0,
  });
  const lipT = 0.0028;
  const lipH = 0.0014;
  const lipY = H * 0.5 + lipH * 0.35;
  const lipSpecs = [
    { s: [W, lipH, lipT], p: [0, lipY, -D * 0.5 + lipT * 0.5] },
    { s: [W, lipH, lipT], p: [0, lipY, D * 0.5 - lipT * 0.5] },
    { s: [lipT, lipH, D - lipT * 2], p: [-W * 0.5 + lipT * 0.5, lipY, 0] },
    { s: [lipT, lipH, D - lipT * 2], p: [W * 0.5 - lipT * 0.5, lipY, 0] },
  ];
  lipSpecs.forEach(({ s, p }) => {
    const lip = new THREE.Mesh(new THREE.BoxGeometry(s[0], s[1], s[2]), lipMat);
    lip.position.set(p[0], p[1], p[2]);
    lip.castShadow = true;
    lip.receiveShadow = true;
    group.add(lip);
  });

  return group;
}

const siliconeMat = createSiliconeMat();
siliconeMat.position.set(CONFIG.mat.x, -0.034, CONFIG.mat.z);
scene.add(siliconeMat);

/** Blue semicircle aura that sits behind the pad in 3D (true depth, not a DOM overlay). */
function createEndAura() {
  const group = new THREE.Group();
  group.name = 'EndAura';
  group.visible = false;

  const makeGlowTex = () => {
    const size = 512;
    const c = document.createElement('canvas');
    c.width = c.height = size;
    const ctx = c.getContext('2d');
    ctx.clearRect(0, 0, size, size);
    // Flat edge sits on the bottom of the texture so it meets the mat
    const baseY = size;
    const g = ctx.createRadialGradient(
      size * 0.5,
      baseY,
      size * 0.04,
      size * 0.5,
      baseY - size * 0.28,
      size * 0.55,
    );
    g.addColorStop(0, 'rgba(200, 236, 255, 0.95)');
    g.addColorStop(0.3, 'rgba(31, 143, 196, 0.5)');
    g.addColorStop(0.65, 'rgba(31, 143, 196, 0.14)');
    g.addColorStop(1, 'rgba(31, 143, 196, 0)');
    ctx.fillStyle = g;
    ctx.beginPath();
    ctx.moveTo(size * 0.05, baseY);
    ctx.arc(size * 0.5, baseY, size * 0.45, Math.PI, 0, false);
    ctx.closePath();
    ctx.fill();
    const tex = new THREE.CanvasTexture(c);
    tex.colorSpace = THREE.SRGBColorSpace;
    return tex;
  };

  const plate = new THREE.Mesh(
    new THREE.PlaneGeometry(0.28, 0.2),
    new THREE.MeshBasicMaterial({
      map: makeGlowTex(),
      transparent: true,
      opacity: 0,
      depthTest: true,
      depthWrite: false,
      side: THREE.DoubleSide,
    }),
  );
  // Far enough behind the pad; local Y places the flat edge on the mat
  plate.position.set(0, 0.1, -0.05);
  plate.renderOrder = -2;
  group.add(plate);

  group.userData.plate = plate;
  return group;
}

const endAura = createEndAura();
scene.add(endAura);

let needsRender = true;

const parts = { case: null, pcb: null, top: null, buttons: [] };
const cameraBase = new THREE.Vector3(0.08, 0.06, 0.22);
const lookTarget = new THREE.Vector3(0, 0, 0);
const HOME = { pos: [0, 0, 0], rot: [0, 0, 0] };
const assemblyHome = new THREE.Vector3();
/** Assembly Y while PCB stands alone (before shell) so the board sits on the mat. */
const assemblyKeysY = { value: 0 };
/** World-space framing anchor = seated assembly center (set in fitCameraBaseline). */
const focusAnchor = new THREE.Vector3(CONFIG.mat.x, 0.02, CONFIG.mat.z + 0.012);
let assemblyHomeReady = false;

/** Classify export material by base color for theme-friendly retints. */
function classifyMatColor(hex) {
  const r = ((hex >> 16) & 255) / 255;
  const g = ((hex >> 8) & 255) / 255;
  const b = (hex & 255) / 255;
  const max = Math.max(r, g, b);
  const min = Math.min(r, g, b);
  const sat = max === 0 ? 0 : (max - min) / max;
  const lum = 0.2126 * r + 0.7152 * g + 0.0722 * b;

  // Sage / FR4 greens from the Fusion export
  if (g > r * 1.05 && g > b * 0.95 && sat > 0.08 && lum > 0.25 && lum < 0.75) {
    return 'fr4';
  }
  // Bright / off-white plastics
  if (lum > 0.72 && sat < 0.18) return 'plastic';
  // Mid silver / aluminium
  if (lum > 0.45 && lum <= 0.72 && sat < 0.12) return 'metal';
  // Dark housings / chips
  if (lum <= 0.45 && sat < 0.2) return 'dark';
  // Cool blue-greys (XIAO silkscreen-ish)
  if (b > r && lum > 0.55 && sat < 0.25) return 'module';
  return 'other';
}

function enhanceMaterial(mat, role) {
  if (!mat || !mat.color) return;
  mat.side = THREE.DoubleSide;

  const hex = mat.color.getHex();
  const kind = classifyMatColor(hex);

  if (role === 'pcb') {
    if (kind === 'fr4') {
      // Richer solder-mask green that pops on the light table
      mat.color.setHex(0x3d8f6e);
      mat.roughness = 0.78;
      mat.metalness = 0.06;
    } else if (kind === 'metal' || kind === 'module') {
      mat.color.setHex(kind === 'module' ? 0xd8dde8 : 0x9aa1a8);
      mat.roughness = kind === 'module' ? 0.42 : 0.36;
      mat.metalness = kind === 'module' ? 0.55 : 0.72;
    } else if (kind === 'dark') {
      mat.color.setHex(0x2a2e32);
      mat.roughness = 0.65;
      mat.metalness = 0.2;
    } else if (kind === 'plastic') {
      mat.color.setHex(0xf2f4f6);
      mat.roughness = 0.55;
      mat.metalness = 0.08;
    }
  } else if (role === 'case' || role === 'top') {
    if (kind === 'plastic' || kind === 'other' || kind === 'module') {
      mat.color.setHex(0xf4f5f7);
      mat.roughness = 0.52;
      mat.metalness = 0.05;
    } else if (kind === 'dark') {
      mat.color.setHex(0x3a3f44);
      mat.roughness = 0.6;
      mat.metalness = 0.12;
    } else if (kind === 'metal') {
      mat.color.setHex(0xb8bec4);
      mat.roughness = 0.4;
      mat.metalness = 0.45;
    }
  } else if (role === 'button') {
    if (kind === 'plastic' || kind === 'other' || kind === 'module') {
      mat.color.setHex(0xf7f8fa);
      mat.roughness = 0.48;
      mat.metalness = 0.04;
    } else if (kind === 'metal') {
      mat.color.setHex(0xaeb4ba);
      mat.roughness = 0.38;
      mat.metalness = 0.5;
    } else if (kind === 'dark') {
      mat.color.setHex(0x33383d);
      mat.roughness = 0.55;
      mat.metalness = 0.15;
    }
  }

  if (typeof mat.envMapIntensity === 'number' || 'envMapIntensity' in mat) {
    mat.envMapIntensity = 0.5;
  }
  mat.needsUpdate = true;
}

function applyMetalGrey(mat) {
  if (!mat || !mat.color) return;
  mat.side = THREE.DoubleSide;
  // Clear baked-white maps so the metal tint actually reads
  if (mat.map) mat.map = null;
  if (mat.emissiveMap) mat.emissiveMap = null;
  if (mat.color) mat.color.setHex(0x6a727c);
  if ('emissive' in mat && mat.emissive) mat.emissive.setHex(0x000000);
  mat.roughness = 0.32;
  mat.metalness = 0.88;
  if (typeof mat.envMapIntensity === 'number' || 'envMapIntensity' in mat) {
    mat.envMapIntensity = 0.85;
  }
  mat.needsUpdate = true;
}

function isUsbConnector(mesh) {
  if (mesh?.userData?.usbConnector) return true;
  let node = mesh;
  while (node) {
    const n = (node.name || '').toUpperCase();
    if (
      n.includes('TYPE-C')
      || n.includes('TYPE_C')
      || n.includes('TYPEC')
      || n.startsWith('TYPE')
      || n.includes('USB')
    ) {
      return true;
    }
    node = node.parent;
  }
  return false;
}

function prepareMeshes(root, renderOrder = 0, role = 'other') {
  root.traverse((child) => {
    if (!child.isMesh) return;
    child.castShadow = true;
    child.receiveShadow = true;
    child.renderOrder = renderOrder;
    // Clone so TYPE-C metal tint isn't overwritten by shared plastic mats
    if (Array.isArray(child.material)) {
      child.material = child.material.map((m) => (m ? m.clone() : m));
    } else if (child.material) {
      child.material = child.material.clone();
    }
    const mats = Array.isArray(child.material) ? child.material : [child.material];
    const usb = isUsbConnector(child);
    mats.forEach((mat) => {
      if (usb) applyMetalGrey(mat);
      else enhanceMaterial(mat, role);
    });
  });
}

function partKind(name) {
  const n = (name || '').toUpperCase();
  if (n.includes('CASE')) return 'case';
  if (n.includes('TOP')) return 'top';
  if (n.includes('BUTTON') || n.includes('TS16')) return 'button';
  if (
    n.includes('OASIS_HW_PCB')
    || n.includes('TYPE-C')
    || n.includes('BOSS')
    || n.includes('XIAO')
  ) {
    return 'pcb';
  }
  return null;
}

function bucketMeshes(root) {
  const buckets = { case: [], pcb: [], top: [], button: [] };
  root.traverse((obj) => {
    if (!obj.isMesh) return;
    let node = obj;
    let kind = null;
    while (node) {
      const n = (node.name || '').toUpperCase();
      if (n.startsWith('CASE')) { kind = 'case'; break; }
      if (n.startsWith('TOP')) { kind = 'top'; break; }
      if (n.startsWith('BUTTON') || n.startsWith('TS16')) { kind = 'button'; break; }
      if (n.startsWith('OASIS_HW') || n.startsWith('TYPE') || n.startsWith('BOSS') || n.includes('XIAO')) {
        kind = 'pcb';
        break;
      }
      const k = partKind(node.name);
      if (k) { kind = k; break; }
      node = node.parent;
    }
    buckets[kind || 'pcb'].push(obj);
  });
  return buckets;
}

function makeGroupFromMeshes(meshes) {
  const group = new THREE.Group();
  meshes.forEach((mesh) => {
    // Tag USB before detaching — parent node names are lost after reparent
    let node = mesh;
    while (node) {
      const n = (node.name || '').toUpperCase();
      if (
        n.includes('TYPE-C')
        || n.includes('TYPE_C')
        || n.includes('TYPEC')
        || n.startsWith('TYPE')
        || n.includes('USB')
      ) {
        mesh.userData.usbConnector = true;
        break;
      }
      node = node.parent;
    }
    mesh.updateWorldMatrix(true, false);
    if (mesh.parent) mesh.parent.remove(mesh);
    group.attach(mesh);
  });
  return group;
}

function clusterButtons(buttonMeshes) {
  if (!buttonMeshes.length) return [];
  const entries = buttonMeshes.map((mesh) => {
    const box = new THREE.Box3().setFromObject(mesh);
    return { mesh, y: box.getCenter(new THREE.Vector3()).y };
  });
  entries.sort((a, b) => b.y - a.y);
  const ys = entries.map((e) => e.y);
  const ymin = Math.min(...ys);
  const ymax = Math.max(...ys);
  const span = Math.max(ymax - ymin, 0.001);
  const bands = [[], [], []];
  entries.forEach((e) => {
    const t = (e.y - ymin) / span;
    const idx = t > 0.66 ? 0 : t > 0.33 ? 1 : 2;
    bands[idx].push(e.mesh);
  });
  if (bands.filter((b) => b.length).length < 2) {
    const n = Math.ceil(entries.length / 3);
    return [
      makeGroupFromMeshes(entries.slice(0, n).map((e) => e.mesh)),
      makeGroupFromMeshes(entries.slice(n, n * 2).map((e) => e.mesh)),
      makeGroupFromMeshes(entries.slice(n * 2).map((e) => e.mesh)),
    ].filter((g) => g.children.length);
  }
  return bands.filter((b) => b.length).map((b) => makeGroupFromMeshes(b));
}

function easeOutCubic(t) {
  return 1 - Math.pow(1 - t, 3);
}

function easeInOutCubic(t) {
  const x = Math.max(0, Math.min(1, t));
  return x < 0.5 ? 4 * x * x * x : 1 - Math.pow(-2 * x + 2, 3) / 2;
}

function lerpPose(group, startCfg, t, home = HOME) {
  const e = easeOutCubic(Math.max(0, Math.min(1, t)));
  group.position.set(
    startCfg.pos[0] * (1 - e) + home.pos[0] * e,
    startCfg.pos[1] * (1 - e) + home.pos[1] * e,
    startCfg.pos[2] * (1 - e) + home.pos[2] * e,
  );
  group.rotation.set(
    startCfg.rot[0] * (1 - e) + home.rot[0] * e,
    startCfg.rot[1] * (1 - e) + home.rot[1] * e,
    startCfg.rot[2] * (1 - e) + home.rot[2] * e,
  );
}

/** Straight lerp between two poses (no assumed HOME). */
function lerpPoseBetween(group, fromCfg, toCfg, t, ease = easeInOutCubic) {
  const e = ease(Math.max(0, Math.min(1, t)));
  group.position.set(
    fromCfg.pos[0] * (1 - e) + toCfg.pos[0] * e,
    fromCfg.pos[1] * (1 - e) + toCfg.pos[1] * e,
    fromCfg.pos[2] * (1 - e) + toCfg.pos[2] * e,
  );
  group.rotation.set(
    fromCfg.rot[0] * (1 - e) + toCfg.rot[0] * e,
    fromCfg.rot[1] * (1 - e) + toCfg.rot[1] * e,
    fromCfg.rot[2] * (1 - e) + toCfg.rot[2] * e,
  );
}

/** Arc over the device so side→home paths clear the PCB / keys. */
function lerpPoseArc(group, fromCfg, toCfg, t, lift = 0.04) {
  const e = easeInOutCubic(Math.max(0, Math.min(1, t)));
  const yArc = Math.sin(e * Math.PI) * lift;
  group.position.set(
    fromCfg.pos[0] * (1 - e) + toCfg.pos[0] * e,
    fromCfg.pos[1] * (1 - e) + toCfg.pos[1] * e + yArc,
    fromCfg.pos[2] * (1 - e) + toCfg.pos[2] * e,
  );
  group.rotation.set(
    fromCfg.rot[0] * (1 - e) + toCfg.rot[0] * e,
    fromCfg.rot[1] * (1 - e) + toCfg.rot[1] * e,
    fromCfg.rot[2] * (1 - e) + toCfg.rot[2] * e,
  );
}

function setPose(group, cfg) {
  group.position.set(...cfg.pos);
  group.rotation.set(...cfg.rot);
}

function riseOf(cfg, dy = 0.07) {
  return {
    pos: [cfg.pos[0], cfg.pos[1] + dy, cfg.pos[2]],
    rot: [cfg.rot[0], cfg.rot[1], cfg.rot[2]],
  };
}

/** desk → rise → park (hold) → approach → home. Stays clear of the PCB volume. */
function flyClear(group, deskCfg, parkCfg, approachCfg, t) {
  const x = Math.max(0, Math.min(1, t));
  if (x < 0.14) {
    lerpPoseBetween(group, deskCfg, riseOf(deskCfg, 0.085), x / 0.14);
  } else if (x < 0.36) {
    lerpPoseBetween(group, riseOf(deskCfg, 0.085), parkCfg, (x - 0.14) / 0.22);
  } else if (x < 0.5) {
    setPose(group, parkCfg); // hold in clear view
  } else if (x < 0.72) {
    lerpPoseBetween(group, parkCfg, approachCfg, (x - 0.5) / 0.22);
  } else {
    lerpPoseBetween(group, approachCfg, HOME, (x - 0.72) / 0.28);
  }
}

function cameraForProgress(p) {
  // Offsets from focusAnchor (seated pad center). Assembled shots sit slightly
  // above eye-line so we read the button face, not the underside.
  const narrow = window.innerWidth < 780;
  const fx = focusAnchor.x;
  const fy = focusAnchor.y;
  const fz = focusAnchor.z;

  const overview = narrow
    ? { pos: [0.055, 0.15, 0.38], look: [0, -0.018, 0] }
    : { pos: [0.045, 0.13, 0.34], look: [0, -0.015, 0] };
  const boardFocus = narrow
    ? { pos: [0.06, 0.075, 0.30], look: [0, -0.005, 0] }
    : { pos: [0.055, 0.065, 0.28], look: [0, -0.004, 0] };
  // Front 3/4 product shot: eye-level, closer so the pad fills more of the frame
  const detail = narrow
    ? { pos: [0.06, 0.02, 0.255], look: [0, 0.008, 0] }
    : { pos: [0.055, 0.016, 0.24], look: [0, 0.008, 0] };
  const hero = narrow
    ? { pos: [0.056, 0.018, 0.245], look: [0, 0.01, 0] }
    : { pos: [0.05, 0.014, 0.23], look: [0, 0.01, 0] };
  const demo = narrow
    ? { pos: [0.054, 0.016, 0.25], look: [0, 0.012, 0] }
    : { pos: [0.048, 0.012, 0.235], look: [0, 0.012, 0] };
  const endPop = narrow
    ? { pos: [0.05, 0.045, 0.275], look: [0, 0.02, 0] }
    : { pos: [0.044, 0.04, 0.26], look: [0, 0.02, 0] };

  let from = overview;
  let to = detail;
  let t = 0;

  if (p < 0.10) {
    from = overview;
    to = boardFocus;
    t = p / 0.10;
  } else if (p < 0.20) {
    from = boardFocus;
    to = boardFocus;
    t = 1;
  } else if (p < 0.48) {
    from = boardFocus;
    to = detail;
    t = (p - 0.20) / 0.28;
  } else if (p < 0.62) {
    from = detail;
    to = hero;
    t = (p - 0.48) / 0.14;
  } else if (p < 0.90) {
    from = hero;
    to = demo;
    t = Math.min(1, (p - 0.62) / 0.28);
  } else {
    from = demo;
    to = endPop;
    t = Math.min(1, (p - 0.90) / 0.1);
  }

  const e = easeOutCubic(t);
  cameraBase.set(
    fx + from.pos[0] * (1 - e) + to.pos[0] * e,
    fy + from.pos[1] * (1 - e) + to.pos[1] * e,
    fz + from.pos[2] * (1 - e) + to.pos[2] * e,
  );
  lookTarget.set(
    fx + from.look[0] * (1 - e) + to.look[0] * e,
    fy + from.look[1] * (1 - e) + to.look[1] * e,
    fz + from.look[2] * (1 - e) + to.look[2] * e,
  );
}

/** Shift frustum so optical center sits below the fixed nav (prevents top chop). */
function applyNavSafeView(w, h, endReveal = false) {
  const navCss = getComputedStyle(document.documentElement).getPropertyValue('--nav-h');
  const navH = Math.max(48, parseFloat(navCss) || 68);
  const footerReserve = endReveal ? 130 : 0;
  const fullH = h + navH * 0.35 + footerReserve;
  camera.setViewOffset(w, fullH, 0, 0, w, h);
}

function pressAmount(local) {
  // Rise to full press mid-chapter, ease off slightly at end
  if (local < 0.2) return local / 0.2;
  if (local < 0.75) return 1;
  return Math.max(0.35, 1 - (local - 0.75) / 0.25 * 0.65);
}

function updateAssembly(progress) {
  const p = Math.max(0, Math.min(1, progress));
  const ch = chapterAt(p);
  const keys = chapterById('keys');
  const shell = chapterById('shell');
  const { approach, park } = CONFIG;

  // --- Case: stay on the desk through keys, then fly clear into the back ---
  if (parts.case) {
    if (p < shell.start) {
      setPose(parts.case, CONFIG.desk.case);
      parts.case.visible = true;
    } else {
      parts.case.visible = true;
      flyClear(
        parts.case,
        CONFIG.desk.case,
        park.case,
        approach.case,
        chapterLocal(p, shell),
      );
    }
  }

  // --- PCB: stand up first so keys have a clear landing zone ---
  if (parts.pcb) {
    if (p < keys.start) {
      setPose(parts.pcb, CONFIG.desk.pcb);
    } else if (p < shell.start) {
      const local = chapterLocal(p, keys);
      lerpPose(parts.pcb, CONFIG.desk.pcb, Math.min(1, local / 0.4));
    } else {
      setPose(parts.pcb, HOME);
    }
    parts.pcb.visible = true;
  }

  // --- Buttons: wait, then rise → park in front → slide onto switches ---
  if (parts.buttons.length) {
    parts.buttons.forEach((btn, i) => {
      const deskPose = CONFIG.desk.buttons[i] || CONFIG.desk.buttons[0];
      const parkPose = park.buttons[i] || park.buttons[0];
      if (p < keys.start) {
        setPose(btn, deskPose);
      } else if (p < keys.end) {
        const local = chapterLocal(p, keys);
        const startAt = 0.36 + i * 0.12;
        if (local < startAt) {
          setPose(btn, deskPose);
        } else {
          flyClear(
            btn,
            deskPose,
            parkPose,
            approach.buttons,
            (local - startAt) / Math.max(0.001, 1 - startAt),
          );
        }
      } else {
        setPose(btn, HOME);
      }

      let press = 0;
      if (ch.phase === 'demo' && ch.buttonIndex === i) {
        press = pressAmount(chapterLocal(p, ch));
      }
      if (p >= keys.end) {
        btn.position.z = HOME.pos[2] - CONFIG.pressDepth * press;
      }
      btn.visible = true;
    });
  }

  // --- Top: stay on the desk through keys, then fly clear onto the front ---
  if (parts.top) {
    if (p < shell.start) {
      setPose(parts.top, CONFIG.desk.top);
      parts.top.visible = true;
    } else {
      parts.top.visible = true;
      const local = chapterLocal(p, shell);
      const startAt = 0.1;
      if (local < startAt) {
        setPose(parts.top, CONFIG.desk.top);
      } else {
        flyClear(
          parts.top,
          CONFIG.desk.top,
          park.top,
          approach.top,
          (local - startAt) / (1 - startAt),
        );
      }
    }
  }

  // Sync mock pad highlight (only when demo button changes)
  const demoKey = ch.phase === 'demo' ? ch.buttonIndex : -1;
  if (demoKey !== lastDemoKey) {
    lastDemoKey = demoKey;
    document.body.classList.remove('demo-btn-0', 'demo-btn-1', 'demo-btn-2');
    if (demoKey >= 0) document.body.classList.add(`demo-btn-${demoKey}`);
    document.querySelectorAll('.feature-pad-btn').forEach((el) => {
      const idx = Number(el.dataset.btn);
      el.classList.toggle('active', demoKey === idx);
    });
  }

  cameraForProgress(p);

  // Keep the pad seated on the mat. During keys the PCB is at HOME without the
  // case, so drop the group so the board foot rests on the mat, then rise as
  // the shell closes.
  const endT = p >= 0.90 ? easeOutCubic(Math.min(1, (p - 0.90) / 0.1)) : 0;
  const baseScale = 1.22;
  if (assemblyHomeReady) {
    const pop = endT;
    let y = assemblyHome.y + pop * 0.008;
    if (p >= keys.start && p < shell.end) {
      const keysLocal = chapterLocal(p, keys);
      const pcbPlanted = easeOutCubic(Math.min(1, keysLocal / 0.4));
      const shellT =
        p <= shell.start ? 0 : easeOutCubic(chapterLocal(p, shell));
      const plantedY =
        assemblyKeysY.value * pcbPlanted + assemblyHome.y * (1 - pcbPlanted);
      y = plantedY * (1 - shellT) + assemblyHome.y * shellT + pop * 0.008;
    }
    // Slightly shrink on the end beat so the pad clears the fixed footer
    assembly.scale.setScalar(baseScale * (1 - endT * 0.16));
    assembly.position.set(assemblyHome.x, y, assemblyHome.z);
    assembly.rotation.set(0, 0, 0);

    // Aura group on the real mat top so the semicircle flat edge touches the mat
    let matTop = desk.position.y + CONFIG.mat.h;
    try {
      const base = siliconeMat.children[0];
      if (base) {
        const bb = new THREE.Box3().setFromObject(base);
        matTop = bb.max.y;
      }
    } catch (_) { /* ignore */ }
    endAura.position.set(
      assemblyHome.x,
      matTop,
      assemblyHome.z - 0.02,
    );
    endAura.visible = endT > 0.02;
    const plate = endAura.userData.plate;
    if (plate?.material) plate.material.opacity = 0.92 * endT;
  } else {
    assembly.scale.setScalar(baseScale);
  }

  const parent = canvas.parentElement;
  if (parent) {
    applyNavSafeView(
      parent.clientWidth || window.innerWidth,
      parent.clientHeight || window.innerHeight,
      endT > 0.15,
    );
    camera.updateProjectionMatrix();
  }

  syncOverlay(p);
  needsRender = true;
}

function seatOnDesk(group, rot, slot, deskY, matSurfaceY) {
  group.position.set(0, 0, 0);
  group.rotation.set(rot[0], rot[1], rot[2]);
  group.updateMatrixWorld(true);

  const box = new THREE.Box3().setFromObject(group);
  const center = box.getCenter(new THREE.Vector3());
  const matPad = 0.022;
  const overlapsMat =
    !!slot.onMat ||
    (Math.abs(slot.x - CONFIG.mat.x) < CONFIG.mat.w * 0.5 + matPad &&
      Math.abs(slot.z - CONFIG.mat.z) < CONFIG.mat.d * 0.5 + matPad);
  const surfaceY = overlapsMat ? matSurfaceY : deskY;
  const lift = overlapsMat ? 0.0012 : 0.002;
  group.position.set(
    slot.x - center.x,
    surfaceY - box.min.y + lift,
    slot.z - center.z,
  );
  group.updateMatrixWorld(true);

  return {
    pos: group.position.toArray(),
    rot: [rot[0], rot[1], rot[2]],
  };
}

function computeDeskLayouts(deskY) {
  siliconeMat.updateMatrixWorld(true);
  const matBase = siliconeMat.children[0];
  const matSurfaceY = matBase
    ? new THREE.Box3().setFromObject(matBase).max.y
    : deskY + CONFIG.mat.h;

  // Temporarily seat each part (PCB on mat; others around it), store pose, restore home
  CONFIG.desk.case = seatOnDesk(
    parts.case,
    CONFIG.desk.case.rot,
    CONFIG.deskSlots.case,
    deskY,
    matSurfaceY,
  );
  CONFIG.desk.pcb = seatOnDesk(
    parts.pcb,
    CONFIG.desk.pcb.rot,
    CONFIG.deskSlots.pcb,
    deskY,
    matSurfaceY,
  );
  parts.buttons.forEach((btn, i) => {
    CONFIG.desk.buttons[i] = seatOnDesk(
      btn,
      CONFIG.desk.buttons[i].rot,
      CONFIG.deskSlots.buttons[i] || CONFIG.deskSlots.buttons[0],
      deskY,
      matSurfaceY,
    );
  });
  CONFIG.desk.top = seatOnDesk(
    parts.top,
    CONFIG.desk.top.rot,
    CONFIG.deskSlots.top,
    deskY,
    matSurfaceY,
  );

  // Reset to home so centerAssembly / fitCamera can use assembled state
  [parts.case, parts.pcb, parts.top, ...parts.buttons].forEach((g) => {
    if (!g) return;
    g.position.set(0, 0, 0);
    g.rotation.set(0, 0, 0);
  });
}

function centerAssembly() {
  // Center XZ only; Y is finalized in fitCameraBaseline so the pad sits on the mat
  updateAssembly(1);
  assembly.rotation.set(0, 0, 0);
  assembly.position.set(0, 0, 0);
  assembly.updateMatrixWorld(true);
  const box = new THREE.Box3().setFromObject(assembly);
  const center = box.getCenter(new THREE.Vector3());
  assembly.position.set(
    -center.x + CONFIG.mat.x,
    -center.y,
    -center.z + CONFIG.mat.z + 0.012,
  );
  assemblyHome.copy(assembly.position);
  assemblyHomeReady = false;
  updateAssembly(0);
}

function fitCameraBaseline() {
  const deskY = -0.038;
  desk.position.y = deskY;
  deskRim.position.y = deskY - 0.001;
  deskShadow.position.y = deskY + 0.0005;
  siliconeMat.position.set(
    CONFIG.mat.x,
    deskY + CONFIG.mat.h * 0.5 + 0.0008,
    CONFIG.mat.z,
  );
  siliconeMat.visible = true;
  desk.visible = true;
  deskRim.visible = true;
  deskShadow.visible = true;

  // Seat fully assembled pad onto the mat's real base top (ignore rim lips)
  updateAssembly(1);
  assembly.rotation.set(0, 0, 0);
  assembly.position.copy(assemblyHome);
  assembly.updateMatrixWorld(true);
  let box = new THREE.Box3().setFromObject(assembly);

  siliconeMat.updateMatrixWorld(true);
  const matBase = siliconeMat.children[0];
  const baseBox = matBase
    ? new THREE.Box3().setFromObject(matBase)
    : new THREE.Box3().setFromObject(siliconeMat);
  const seatY = baseBox.max.y + 0.0018;
  assembly.position.y += seatY - box.min.y;
  assemblyHome.copy(assembly.position);
  assemblyHomeReady = true;

  // Keys-chapter seat: PCB alone at HOME sits higher than the case foot.
  // Measure how far to drop the group so the board rests on the mat.
  if (parts.pcb) setPose(parts.pcb, HOME);
  parts.buttons.forEach((btn) => setPose(btn, HOME));
  assembly.updateMatrixWorld(true);
  const boardBox = new THREE.Box3();
  if (parts.pcb) boardBox.expandByObject(parts.pcb);
  parts.buttons.forEach((btn) => boardBox.expandByObject(btn));
  assemblyKeysY.value = Number.isFinite(boardBox.min.y)
    ? assembly.position.y + (seatY - boardBox.min.y)
    : assemblyHome.y;
  // Restore assembled poses before desk layout pass
  [parts.case, parts.pcb, parts.top, ...parts.buttons].forEach((g) => {
    if (!g) return;
    g.position.set(0, 0, 0);
    g.rotation.set(0, 0, 0);
  });

  // Framing must track the seated pad, not the pre-seat origin
  assembly.updateMatrixWorld(true);
  box = new THREE.Box3().setFromObject(assembly);
  box.getCenter(focusAnchor);
  // Bias toward mid-button face for a product read
  focusAnchor.y = box.min.y + (box.max.y - box.min.y) * 0.48;

  computeDeskLayouts(deskY);

  updateAssembly(0);
  cameraForProgress(0);
  camera.position.copy(cameraBase);
  camera.lookAt(lookTarget);
}

async function initModels() {
  const loader = new GLTFLoader();
  const gltf = await new Promise((resolve, reject) => {
    loader.load(CONFIG.modelPath, resolve, undefined, reject);
  });

  const root = gltf.scene;
  root.updateMatrixWorld(true);
  const buckets = bucketMeshes(root);

  parts.case = makeGroupFromMeshes(buckets.case);
  parts.pcb = makeGroupFromMeshes(buckets.pcb);
  parts.top = makeGroupFromMeshes(buckets.top);
  parts.buttons = clusterButtons(buckets.button);

  prepareMeshes(parts.case, 0, 'case');
  prepareMeshes(parts.pcb, 1, 'pcb');
  parts.buttons.forEach((b) => prepareMeshes(b, 2, 'button'));
  prepareMeshes(parts.top, 3, 'top');

  assembly.add(parts.case);
  assembly.add(parts.pcb);
  parts.buttons.forEach((b) => assembly.add(b));
  assembly.add(parts.top);

  centerAssembly();
  fitCameraBaseline();
  updateAssembly(0);

  let usbCount = 0;
  let usbHex = null;
  parts.pcb?.traverse((o) => {
    if (!o.isMesh || !o.userData?.usbConnector) return;
    usbCount += 1;
    if (!usbHex) {
      const m = Array.isArray(o.material) ? o.material[0] : o.material;
      usbHex = m?.color?.getHexString?.() || null;
    }
  });

  window.__OASIS_HOMES = {
    mode: 'workbench',
    meshCounts: {
      case: buckets.case.length,
      pcb: buckets.pcb.length,
      top: buckets.top.length,
      button: buckets.button.length,
      buttonGroups: parts.buttons.length,
      usb: usbCount,
      usbHex,
    },
  };
}

window.__OASIS_SET_PROGRESS = updateAssembly;
window.__OASIS_DEBUG = (progress = 1) => {
  updateAssembly(progress);
  assembly.updateMatrixWorld(true);
  const boxOf = (obj) => {
    if (!obj) return null;
    const b = new THREE.Box3().setFromObject(obj);
    return {
      min: b.min.toArray(),
      max: b.max.toArray(),
      center: b.getCenter(new THREE.Vector3()).toArray(),
    };
  };
  const matTop = desk.position.y + CONFIG.mat.h;
  const asm = boxOf(assembly);
  let seatTop = matTop;
  try {
    const base = siliconeMat.children[0];
    if (base) {
      const bb = new THREE.Box3().setFromObject(base);
      seatTop = bb.max.y;
    }
  } catch (_) { /* ignore */ }
  return {
    chapter: chapterAt(progress),
    homes: window.__OASIS_HOMES,
    camera: cameraBase.toArray(),
    look: lookTarget.toArray(),
    focus: focusAnchor.toArray(),
    deskY: desk.position.y,
    matTop,
    seatTop,
    gap: asm ? asm.min[1] - seatTop : null,
    deskPos: CONFIG.desk,
    boxes: {
      case: boxOf(parts.case),
      pcb: boxOf(parts.pcb),
      top: boxOf(parts.top),
      buttons: parts.buttons.map(boxOf),
      assembly: asm,
    },
  };
};

function resize() {
  const parent = canvas.parentElement;
  if (!parent) return;
  const w = parent.clientWidth || window.innerWidth;
  const h = parent.clientHeight || window.innerHeight;
  if (w < 2 || h < 2) return;
  renderer.setSize(w, h, false);
  camera.aspect = w / h;
  const endReveal = document.body.classList.contains('is-end-reveal');
  applyNavSafeView(w, h, endReveal);
  camera.updateProjectionMatrix();
  needsRender = true;
}

window.addEventListener('resize', resize);
if (window.ResizeObserver) {
  new ResizeObserver(() => resize()).observe(canvas.parentElement);
}

let mouseX = 0;
let mouseY = 0;
let lastDemoKey = -2;
let camSettled = false;

canvas.addEventListener('mousemove', (e) => {
  if (window.matchMedia('(pointer: coarse)').matches || window.innerWidth < 780) return;
  const rect = canvas.getBoundingClientRect();
  mouseX = ((e.clientX - rect.left) / rect.width - 0.5) * 0.045;
  mouseY = ((e.clientY - rect.top) / rect.height - 0.5) * 0.028;
  needsRender = true;
  camSettled = false;
});

window.addEventListener('orientationchange', () => {
  mouseX = 0;
  mouseY = 0;
  setTimeout(resize, 120);
});

function animate() {
  requestAnimationFrame(animate);

  const tx = cameraBase.x + mouseX;
  const ty = cameraBase.y - mouseY;
  const tz = cameraBase.z;
  const dx = tx - camera.position.x;
  const dy = ty - camera.position.y;
  const dz = tz - camera.position.z;
  const moving = Math.abs(dx) + Math.abs(dy) + Math.abs(dz) > 0.00004;

  if (moving || needsRender || !camSettled) {
    camera.position.x += dx * 0.08;
    camera.position.y += dy * 0.08;
    camera.position.z += dz * 0.08;
    camera.lookAt(lookTarget);
    renderer.render(scene, camera);
    needsRender = moving;
    camSettled = !moving;
  }
}

initModels()
  .then(() => {
    resize();
    animate();
    gsap.registerPlugin(ScrollTrigger);
    // Pin only the stage — GSAP adds its own pin-spacer for scroll distance.
    // Do NOT also use a manual tall spacer (that leaves empty page after the story).
    ScrollTrigger.create({
      trigger: '#workbench-stage',
      start: 'top top',
      end: () => `+=${Math.round(window.innerHeight * CONFIG.scrollVh)}`,
      pin: true,
      scrub: 0.45,
      anticipatePin: 1,
      invalidateOnRefresh: true,
      onUpdate: (self) => updateAssembly(self.progress),
      onLeave: () => updateAssembly(1),
      onLeaveBack: () => updateAssembly(0),
    });
    ScrollTrigger.refresh();
  })
  .catch((err) => {
    console.error('Failed to load OASIS_ASM:', err);
    const label = document.getElementById('story-title');
    if (label) label.textContent = 'Model failed to load';
  });
