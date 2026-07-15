import * as THREE from 'three';
import { GLTFLoader } from 'three/addons/loaders/GLTFLoader.js';

const CONFIG = {
  scrollVh: 4,
  buttonSpacing: 0.0162,
  stages: [
    { id: 'case', label: 'CASE //', start: 0.0, end: 0.25 },
    { id: 'pcb', label: 'PCB //', start: 0.2, end: 0.45 },
    { id: 'buttons', label: 'BUTTONS //', start: 0.4, end: 0.7 },
    { id: 'top', label: 'TOP //', start: 0.65, end: 1.0 },
  ],
  parts: {
    case: {
      path: 'assets/models/CASE.gltf',
      start: { pos: [0, -0.06, 0.04], rot: [0.2, 0.4, 0] },
      home: { pos: [0, 0, 0], rot: [0, 0, 0] },
      material: { color: 0xf6f6f4, roughness: 0.55, metalness: 0.04 },
    },
    pcb: {
      path: 'assets/models/OASIS.glb',
      start: { pos: [-0.08, 0.06, 0.08], rot: [0, -0.6, 0.3] },
      home: { pos: [0, 0, 0], rot: [0, 0, 0] },
      modelRot: [0, 0, 0],
      material: { color: 0x4a6741, roughness: 0.75, metalness: 0.1 },
    },
    top: {
      path: 'assets/models/TOP.gltf',
      start: { pos: [0, 0.14, 0], rot: [-0.4, -0.5, 0] },
      home: { pos: [0, 0, 0], rot: [0, 0, 0] },
      material: { color: 0xf6f6f4, roughness: 0.55, metalness: 0.04 },
    },
    button: {
      path: 'assets/models/BUTTON.gltf',
      start: { pos: [0, 0, 0.1], rot: [0.5, 0, 0] },
      home: { pos: [0, 0, 0], rot: [0, 0, 0] },
      material: { color: 0xfafaf8, roughness: 0.45, metalness: 0.06 },
      offsets: [
        { y: 0.0162 },
        { y: 0 },
        { y: -0.0162 },
      ],
    },
  },
};

const canvas = document.getElementById('assembly-canvas');
const labelEl = document.getElementById('assembly-label');
const progressFill = document.getElementById('progress-fill');
const stepDots = document.querySelectorAll('.step-dot');
const spacer = document.getElementById('assembly-spacer');

if (!canvas) throw new Error('Assembly canvas not found');

spacer.style.height = `${CONFIG.scrollVh * 100}vh`;

const scene = new THREE.Scene();
scene.background = new THREE.Color(0xf4fafd);

const camera = new THREE.PerspectiveCamera(38, 1, 0.001, 10);
camera.position.set(0.04, 0.01, 0.14);

const renderer = new THREE.WebGLRenderer({ canvas, antialias: true, alpha: false });
renderer.setPixelRatio(Math.min(window.devicePixelRatio, 2));
renderer.shadowMap.enabled = true;
renderer.shadowMap.type = THREE.PCFSoftShadowMap;
renderer.outputColorSpace = THREE.SRGBColorSpace;
renderer.toneMapping = THREE.ACESFilmicToneMapping;
renderer.toneMappingExposure = 1.05;

const assembly = new THREE.Group();
scene.add(assembly);

const ambient = new THREE.AmbientLight(0xffffff, 0.55);
scene.add(ambient);

const keyLight = new THREE.DirectionalLight(0xffffff, 1.1);
keyLight.position.set(2, 4, 3);
keyLight.castShadow = true;
keyLight.shadow.mapSize.set(1024, 1024);
keyLight.shadow.camera.near = 0.01;
keyLight.shadow.camera.far = 2;
keyLight.shadow.camera.left = -0.15;
keyLight.shadow.camera.right = 0.15;
keyLight.shadow.camera.top = 0.15;
keyLight.shadow.camera.bottom = -0.15;
scene.add(keyLight);

const fillLight = new THREE.DirectionalLight(0xa8bec9, 0.45);
fillLight.position.set(-3, 1, -2);
scene.add(fillLight);

const rimLight = new THREE.DirectionalLight(0x7a99ab, 0.35);
rimLight.position.set(0, -2, -3);
scene.add(rimLight);

const ground = new THREE.Mesh(
  new THREE.PlaneGeometry(2, 2),
  new THREE.ShadowMaterial({ opacity: 0.07 })
);
ground.rotation.x = -Math.PI / 2;
ground.position.y = -0.035;
ground.receiveShadow = true;
scene.add(ground);

const loader = new GLTFLoader();
const parts = { case: null, pcb: null, top: null, buttons: [] };
const cameraBase = new THREE.Vector3();
const lookTarget = new THREE.Vector3(0, 0, 0);

function applyMaterial(root, matConfig) {
  root.traverse((child) => {
    if (!child.isMesh) return;
    child.castShadow = true;
    child.receiveShadow = true;
    child.material = new THREE.MeshStandardMaterial({
      color: matConfig.color,
      roughness: matConfig.roughness,
      metalness: matConfig.metalness,
    });
  });
}

function loadModel(path) {
  return new Promise((resolve, reject) => {
    loader.load(path, (gltf) => resolve(gltf.scene), undefined, reject);
  });
}

function fitCameraToAssembly() {
  updateAssembly(1);
  const box = new THREE.Box3().setFromObject(assembly);
  const size = box.getSize(new THREE.Vector3());
  const center = box.getCenter(new THREE.Vector3());
  const maxDim = Math.max(size.x, size.y, size.z, 0.01);
  const fov = camera.fov * (Math.PI / 180);
  const dist = (maxDim * 1.05) / Math.tan(fov / 2);

  lookTarget.set(center.x + 0.008, center.y, center.z);
  cameraBase.set(center.x + 0.008 + dist * 0.08, center.y + dist * 0.04, center.z + dist * 1.15);
  camera.position.copy(cameraBase);
  camera.lookAt(lookTarget);
  camera.near = dist / 100;
  camera.far = dist * 20;
  camera.updateProjectionMatrix();

  ground.position.y = box.min.y - 0.004;
  updateAssembly(0);
}

function applyModelRotation(model, rot) {
  if (!rot || (rot[0] === 0 && rot[1] === 0 && rot[2] === 0)) return;
  model.rotation.set(rot[0], rot[1], rot[2]);
  model.updateMatrixWorld(true);
}

function box3WithOffset(box, offset) {
  const b = box.clone();
  b.min.add(offset);
  b.max.add(offset);
  return b;
}

function overflowScore(caseBox, partBox, pos) {
  const fitted = box3WithOffset(partBox, pos);
  const margin = 0.0012;
  let score = 0;
  score += Math.max(0, caseBox.min.x + margin - fitted.min.x);
  score += Math.max(0, fitted.max.x - (caseBox.max.x - margin));
  score += Math.max(0, caseBox.min.y + margin - fitted.min.y);
  score += Math.max(0, fitted.max.y - (caseBox.max.y - margin));
  score += Math.max(0, caseBox.min.z + margin - fitted.min.z);
  score += Math.max(0, fitted.max.z - (caseBox.max.z - margin));
  return score;
}

function vec3ToArr(v) {
  return [v.x, v.y, v.z];
}

function fitPcbToCase(caseModel, pcbModel) {
  const caseBox = new THREE.Box3().setFromObject(caseModel);
  const caseSize = caseBox.getSize(new THREE.Vector3());
  const caseC = caseBox.getCenter(new THREE.Vector3());
  const inset = 0.0018;
  const overrides = window.__OASIS_OVERRIDES || {};
  const rotations = [
    [-Math.PI / 2, 0, Math.PI / 2],
    [-Math.PI / 2, 0, -Math.PI / 2],
    [Math.PI / 2, 0, Math.PI / 2],
    [Math.PI / 2, 0, 0],
    [-Math.PI / 2, 0, 0],
    [0, 0, Math.PI / 2],
  ];

  if (overrides.pcbRot === 'rx90') rotations.unshift([Math.PI / 2, 0, 0]);
  if (overrides.pcbRot === 'rx-90-rz90') rotations.unshift([-Math.PI / 2, 0, Math.PI / 2]);

  let best = null;

  for (const rot of rotations) {
    const probe = pcbModel.clone(true);
    probe.rotation.set(rot[0], rot[1], rot[2]);
    probe.updateMatrixWorld(true);

    let pcbBox = new THREE.Box3().setFromObject(probe);
    let pcbSize = pcbBox.getSize(new THREE.Vector3());
    const scale = Math.min(
      (caseSize.x - inset * 2) / pcbSize.x,
      (caseSize.y - inset * 2) / pcbSize.y,
      (caseSize.z - inset * 2) / pcbSize.z,
      1,
    );

    if (scale < 0.999) {
      probe.scale.multiplyScalar(scale);
      probe.updateMatrixWorld(true);
      pcbBox = new THREE.Box3().setFromObject(probe);
      pcbSize = pcbBox.getSize(new THREE.Vector3());
    }

    const pcbC = pcbBox.getCenter(new THREE.Vector3());
    const pos = new THREE.Vector3(
      caseC.x - pcbC.x,
      caseC.y - pcbC.y + 0.006,
      caseBox.min.z + inset + pcbSize.z * 0.5 - pcbC.z,
    );

    const overflow = overflowScore(caseBox, pcbBox, pos);
    const score = overflow * 100 + Math.abs(pcbSize.x - caseSize.x) * 0.05;
    if (!best || score < best.score) {
      best = { rot, pos: vec3ToArr(pos), scale, score, overflow };
    }
  }

  return best;
}

function computeHomes(caseModel, topModel, pcbModel, buttonModel) {
  const caseBox = new THREE.Box3().setFromObject(caseModel);
  const caseC = caseBox.getCenter(new THREE.Vector3());
  const inset = 0.0012;

  const topBox = new THREE.Box3().setFromObject(topModel);
  const topC = topBox.getCenter(new THREE.Vector3());

  const btnBox = new THREE.Box3().setFromObject(buttonModel);
  const btnC = btnBox.getCenter(new THREE.Vector3());
  const btnSize = btnBox.getSize(new THREE.Vector3());

  const overrides = window.__OASIS_OVERRIDES || {};
  const stackY = caseBox.max.y - topBox.min.y;
  const topY = overrides.topY ?? stackY;
  const topZ = overrides.topZ ?? (caseC.z - topC.z);

  const topHome = {
    pos: vec3ToArr(new THREE.Vector3(
      caseC.x - topC.x,
      topY,
      topZ,
    )),
    rot: [0, 0, 0],
  };

  const buttonSpacing = overrides.buttonSpacing ?? (btnSize.y * 1.05);
  const buttonHome = {
    pos: vec3ToArr(new THREE.Vector3(
      caseC.x - btnC.x,
      caseC.y - btnC.y + 0.008,
      caseBox.max.z - inset - btnBox.max.z,
    )),
    rot: [0, 0, 0],
  };

  const buttonOffsets = [
    { y: buttonSpacing },
    { y: 0 },
    { y: -buttonSpacing },
  ];
  const pcb = fitPcbToCase(caseModel, pcbModel);

  return { topHome, buttonHome, buttonOffsets, pcb, caseBox, topBox, btnBox };
}

function reportMetrics() {
  if (!parts.case || !parts.top || !parts.pcb) return null;

  updateAssembly(1);
  assembly.updateMatrixWorld(true);

  const caseBox = new THREE.Box3().setFromObject(parts.case);
  const topBox = new THREE.Box3().setFromObject(parts.top);
  const pcbBox = new THREE.Box3().setFromObject(parts.pcb);
  const btnBox = parts.buttons.length
    ? new THREE.Box3().setFromObject(parts.buttons[1] || parts.buttons[0])
    : null;

  const shellGapY = topBox.min.y - caseBox.max.y;
  const topGapX = Math.abs(topBox.getCenter(new THREE.Vector3()).x - caseBox.getCenter(new THREE.Vector3()).x);
  const pcbOverflowX = Math.max(0, pcbBox.max.x - caseBox.max.x)
    + Math.max(0, caseBox.min.x - pcbBox.min.x);

  const metrics = { shellGapY, topGapX, pcbOverflowX };
  window.__OASIS_METRICS = metrics;
  updateAssembly(0);
  return metrics;
}

function normalizePcbScale(caseModel, pcbModel) {
  const caseBox = new THREE.Box3().setFromObject(caseModel);
  const pcbBox = new THREE.Box3().setFromObject(pcbModel);
  const caseSize = caseBox.getSize(new THREE.Vector3());
  const pcbSize = pcbBox.getSize(new THREE.Vector3());
  const caseMax = Math.max(caseSize.x, caseSize.y, caseSize.z);
  const pcbMax = Math.max(pcbSize.x, pcbSize.y, pcbSize.z);
  if (pcbMax > caseMax * 3) {
    const scale = caseMax / pcbMax;
    pcbModel.scale.setScalar(scale);
  }
}

async function initModels() {
  const [caseModel, pcbModel, topModel, buttonModel] = await Promise.all([
    loadModel(CONFIG.parts.case.path),
    loadModel(CONFIG.parts.pcb.path),
    loadModel(CONFIG.parts.top.path),
    loadModel(CONFIG.parts.button.path),
  ]);

  applyMaterial(caseModel, CONFIG.parts.case.material);
  applyMaterial(pcbModel, CONFIG.parts.pcb.material);
  applyMaterial(topModel, CONFIG.parts.top.material);

  applyHomes(caseModel, pcbModel, topModel, buttonModel);

  parts.case = new THREE.Group();
  parts.case.add(caseModel);
  assembly.add(parts.case);

  parts.pcb = new THREE.Group();
  parts.pcb.add(pcbModel);
  parts.pcb.visible = false;
  assembly.add(parts.pcb);

  parts.top = new THREE.Group();
  parts.top.add(topModel);
  parts.top.visible = false;
  assembly.add(parts.top);

  const btnTemplate = buttonModel.clone(true);
  applyMaterial(btnTemplate, CONFIG.parts.button.material);

  CONFIG.parts.button.offsets.forEach((offset) => {
    const btn = new THREE.Group();
    btn.add(btnTemplate.clone(true));
    btn.userData.homeY = offset.y || 0;
    btn.visible = false;
    parts.buttons.push(btn);
    assembly.add(btn);
  });

  centerAssembly();
  fitCameraToAssembly();
  setInitialState();
  reportMetrics();
}

function applyHomes(caseModel, pcbModel, topModel, buttonModel) {
  const homes = computeHomes(caseModel, topModel, pcbModel, buttonModel);
  CONFIG.parts.top.home = homes.topHome;
  CONFIG.parts.button.home = homes.buttonHome;
  CONFIG.parts.button.offsets = homes.buttonOffsets;
  CONFIG.parts.pcb.home = { pos: homes.pcb.pos, rot: [0, 0, 0] };
  CONFIG.parts.pcb.modelRot = homes.pcb.rot;

  pcbModel.rotation.set(0, 0, 0);
  pcbModel.scale.set(1, 1, 1);
  applyModelRotation(pcbModel, CONFIG.parts.pcb.modelRot);
  if (homes.pcb.scale && homes.pcb.scale < 0.999) {
    pcbModel.scale.multiplyScalar(homes.pcb.scale);
    pcbModel.updateMatrixWorld(true);
  }

  window.__OASIS_HOMES = {
    top: CONFIG.parts.top.home,
    pcb: homes.pcb,
    button: CONFIG.parts.button.home,
  };
}

window.__OASIS_REBUILD = async () => {
  while (assembly.children.length) assembly.remove(assembly.children[0]);
  parts.case = null;
  parts.pcb = null;
  parts.top = null;
  parts.buttons = [];
  await initModels();
  ScrollTrigger.refresh();
  reportMetrics();
};

function centerAssembly() {
  updateAssembly(1);
  const box = new THREE.Box3().setFromObject(assembly);
  const center = box.getCenter(new THREE.Vector3());
  assembly.position.sub(center);
  assembly.position.y += 0.005;
  updateAssembly(0);
}

function easeOutCubic(t) {
  return 1 - Math.pow(1 - t, 3);
}

function lerpPart(group, startCfg, t, home = { pos: [0, 0, 0], rot: [0, 0, 0] }) {
  const e = easeOutCubic(Math.max(0, Math.min(1, t)));
  group.position.set(
    startCfg.pos[0] * (1 - e) + home.pos[0] * e,
    startCfg.pos[1] * (1 - e) + home.pos[1] * e,
    startCfg.pos[2] * (1 - e) + home.pos[2] * e
  );
  group.rotation.set(
    startCfg.rot[0] * (1 - e) + home.rot[0] * e,
    startCfg.rot[1] * (1 - e) + home.rot[1] * e,
    startCfg.rot[2] * (1 - e) + home.rot[2] * e
  );
  group.visible = true;
}

function stageProgress(global, stage) {
  return (global - stage.start) / (stage.end - stage.start);
}

function updateAssembly(progress) {
  const p = Math.max(0, Math.min(1, progress));

  if (parts.case) {
    const t = stageProgress(p, CONFIG.stages[0]);
    lerpPart(parts.case, CONFIG.parts.case.start, t, CONFIG.parts.case.home);
    parts.case.visible = p >= CONFIG.stages[0].start;
  }

  if (parts.pcb) {
    const t = stageProgress(p, CONFIG.stages[1]);
    lerpPart(parts.pcb, CONFIG.parts.pcb.start, t, CONFIG.parts.pcb.home);
    parts.pcb.visible = p >= CONFIG.stages[1].start;
  }

  if (parts.buttons.length) {
    const btnP = stageProgress(p, CONFIG.stages[2]);
    const buttonsActive = p >= CONFIG.stages[2].start;
    parts.buttons.forEach((btn, i) => {
      const stagger = i * 0.1;
      const localP = Math.max(0, (btnP - stagger) / (1 - stagger));
      const start = {
        pos: [
          CONFIG.parts.button.start.pos[0] + (i - 1) * 0.025,
          CONFIG.parts.button.start.pos[1],
          CONFIG.parts.button.start.pos[2] + i * 0.02,
        ],
        rot: [...CONFIG.parts.button.start.rot],
      };
      lerpPart(btn, start, localP, {
        pos: [
          CONFIG.parts.button.home.pos[0],
          CONFIG.parts.button.home.pos[1] + btn.userData.homeY,
          CONFIG.parts.button.home.pos[2],
        ],
        rot: CONFIG.parts.button.home.rot,
      });
      btn.visible = buttonsActive && (localP > 0.01 || p > 0.92);
    });
  }

  if (parts.top) {
    const t = stageProgress(p, CONFIG.stages[3]);
    lerpPart(parts.top, CONFIG.parts.top.start, t, CONFIG.parts.top.home);
    parts.top.visible = p >= CONFIG.stages[3].start;
  }

  const rotY = p * 0.06;
  assembly.rotation.y = rotY;

  let activeStage = CONFIG.stages[0];
  for (const stage of CONFIG.stages) {
    if (p >= stage.start) activeStage = stage;
  }
  if (labelEl) labelEl.textContent = activeStage.label;
  if (progressFill) progressFill.style.width = `${p * 100}%`;

  stepDots.forEach((dot, i) => {
    dot.classList.toggle('active', p >= CONFIG.stages[i].start);
  });
}

function setInitialState() {
  updateAssembly(0);
}

window.__OASIS_SET_PROGRESS = updateAssembly;

window.__OASIS_DEBUG = () => {
  updateAssembly(1);
  const box = new THREE.Box3().setFromObject(assembly);
  return {
    assemblyPos: assembly.position.toArray(),
    assemblyRot: assembly.rotation.toArray(),
    boxMin: box.min.toArray(),
    boxMax: box.max.toArray(),
    boxSize: box.getSize(new THREE.Vector3()).toArray(),
    cameraPos: camera.position.toArray(),
    cameraBase: cameraBase.toArray(),
    lookTarget: lookTarget.toArray(),
    partsVisible: {
      case: parts.case?.visible,
      pcb: parts.pcb?.visible,
      top: parts.top?.visible,
      buttons: parts.buttons.map((b) => b.visible),
    },
    partPositions: {
      case: parts.case?.position.toArray(),
      pcb: parts.pcb?.position.toArray(),
      top: parts.top?.position.toArray(),
      buttons: parts.buttons.map((b) => b.position.toArray()),
    },
  };
};

function resize() {
  const parent = canvas.parentElement;
  if (!parent) return;
  const w = parent.clientWidth || window.innerWidth;
  const h = parent.clientHeight || window.innerHeight;
  if (w < 2 || h < 2) return;
  renderer.setSize(w, h, true);
  camera.aspect = w / h;
  camera.updateProjectionMatrix();
}

window.addEventListener('resize', resize);
if (window.ResizeObserver) {
  new ResizeObserver(() => resize()).observe(canvas.parentElement);
}

let mouseX = 0;
let mouseY = 0;

canvas.addEventListener('mousemove', (e) => {
  const rect = canvas.getBoundingClientRect();
  mouseX = ((e.clientX - rect.left) / rect.width - 0.5) * 0.15;
  mouseY = ((e.clientY - rect.top) / rect.height - 0.5) * 0.1;
});

function animate() {
  requestAnimationFrame(animate);
  const targetX = cameraBase.x + mouseX;
  const targetY = cameraBase.y - mouseY;
  camera.position.x += (targetX - camera.position.x) * 0.05;
  camera.position.y += (targetY - camera.position.y) * 0.05;
  camera.position.z += (cameraBase.z - camera.position.z) * 0.05;
  camera.lookAt(lookTarget);
  renderer.render(scene, camera);
}

initModels().then(() => {
  resize();
  animate();

  gsap.registerPlugin(ScrollTrigger);

  ScrollTrigger.create({
    trigger: '#assembly-wrapper',
    start: 'top top',
    end: `+=${CONFIG.scrollVh * 100}%`,
    pin: '#assembly-stage',
    scrub: 0.6,
    onUpdate: (self) => updateAssembly(self.progress),
  });
}).catch((err) => {
  console.error('Failed to load 3D models:', err);
  if (labelEl) labelEl.textContent = 'MODELS LOADING…';
});
