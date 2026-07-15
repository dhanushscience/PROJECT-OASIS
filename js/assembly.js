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
      start: { pos: [0, -0.12, 0], rot: [0.3, 0.6, 0] },
      material: { color: 0xf6f6f4, roughness: 0.55, metalness: 0.04 },
    },
    pcb: {
      path: 'assets/models/OASIS.glb',
      start: { pos: [-0.1, 0, 0.06], rot: [0, -0.8, 0.2] },
      material: { color: 0x4a6741, roughness: 0.75, metalness: 0.1 },
    },
    top: {
      path: 'assets/models/TOP.gltf',
      start: { pos: [0, 0.14, 0], rot: [-0.4, -0.5, 0] },
      material: { color: 0xf6f6f4, roughness: 0.55, metalness: 0.04 },
    },
    button: {
      path: 'assets/models/BUTTON.gltf',
      start: { pos: [0, 0, 0.1], rot: [0.5, 0, 0] },
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

  parts.case = new THREE.Group();
  parts.case.add(caseModel);
  assembly.add(parts.case);

  parts.pcb = new THREE.Group();
  parts.pcb.add(pcbModel);
  assembly.add(parts.pcb);

  parts.top = new THREE.Group();
  parts.top.add(topModel);
  assembly.add(parts.top);

  const btnTemplate = buttonModel.clone(true);
  applyMaterial(btnTemplate, CONFIG.parts.button.material);

  CONFIG.parts.button.offsets.forEach((offset) => {
    const btn = new THREE.Group();
    btn.add(btnTemplate.clone(true));
    btn.userData.homeY = offset.y || 0;
    parts.buttons.push(btn);
    assembly.add(btn);
  });

  centerAssembly();
  setInitialState();
}

function centerAssembly() {
  const box = new THREE.Box3().setFromObject(assembly);
  const center = box.getCenter(new THREE.Vector3());
  assembly.position.sub(center);
  assembly.position.y += 0.005;
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
  group.visible = t > 0.01 || e > 0.01;
}

function stageProgress(global, stage) {
  return (global - stage.start) / (stage.end - stage.start);
}

function updateAssembly(progress) {
  const p = Math.max(0, Math.min(1, progress));

  if (parts.case) lerpPart(parts.case, CONFIG.parts.case.start, stageProgress(p, CONFIG.stages[0]));
  if (parts.pcb) lerpPart(parts.pcb, CONFIG.parts.pcb.start, stageProgress(p, CONFIG.stages[1]));

  if (parts.buttons.length) {
    const btnP = stageProgress(p, CONFIG.stages[2]);
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
      lerpPart(btn, start, localP, { pos: [0, btn.userData.homeY, 0], rot: [0, 0, 0] });
    });
  }

  if (parts.top) lerpPart(parts.top, CONFIG.parts.top.start, stageProgress(p, CONFIG.stages[3]));

  const rotY = p * 0.35;
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

function resize() {
  const parent = canvas.parentElement;
  const w = parent.clientWidth;
  const h = parent.clientHeight;
  renderer.setSize(w, h, false);
  camera.aspect = w / h;
  camera.updateProjectionMatrix();
}

window.addEventListener('resize', resize);

let mouseX = 0;
let mouseY = 0;

canvas.addEventListener('mousemove', (e) => {
  const rect = canvas.getBoundingClientRect();
  mouseX = ((e.clientX - rect.left) / rect.width - 0.5) * 0.15;
  mouseY = ((e.clientY - rect.top) / rect.height - 0.5) * 0.1;
});

function animate() {
  requestAnimationFrame(animate);
  camera.position.x += (0.04 + mouseX - camera.position.x) * 0.05;
  camera.position.y += (0.01 - mouseY - camera.position.y) * 0.05;
  camera.lookAt(0, 0, 0);
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
