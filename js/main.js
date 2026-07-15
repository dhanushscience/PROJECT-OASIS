/* PROJECT OASIS — Landing Page Scripts */

(function () {
  'use strict';

  // ── Navigation scroll state ──
  const nav = document.getElementById('nav');
  window.addEventListener('scroll', () => {
    nav.classList.toggle('scrolled', window.scrollY > 40);
  }, { passive: true });

  // ── Cursor glow ──
  const glow = document.querySelector('.cursor-glow');
  if (glow && window.matchMedia('(pointer: fine)').matches) {
    document.addEventListener('mousemove', (e) => {
      glow.style.left = e.clientX + 'px';
      glow.style.top = e.clientY + 'px';
    }, { passive: true });
  } else if (glow) {
    glow.style.display = 'none';
  }

  // ── Scroll reveal ──
  const revealEls = document.querySelectorAll('.reveal');
  const revealObserver = new IntersectionObserver((entries) => {
    entries.forEach((entry) => {
      if (entry.isIntersecting) {
        entry.target.classList.add('visible');
        revealObserver.unobserve(entry.target);
      }
    });
  }, { threshold: 0.15, rootMargin: '0px 0px -40px 0px' });

  revealEls.forEach((el) => revealObserver.observe(el));

  // ── Parallax on scroll ──
  const parallaxEls = document.querySelectorAll('[data-parallax]');
  let ticking = false;

  function updateParallax() {
    const scrollY = window.scrollY;
    parallaxEls.forEach((el) => {
      const speed = parseFloat(el.dataset.parallax) || 0.1;
      const rect = el.getBoundingClientRect();
      const center = rect.top + rect.height / 2;
      const viewportCenter = window.innerHeight / 2;
      const offset = (center - viewportCenter) * speed;
      el.style.transform = `translateY(${offset}px)`;
    });
    ticking = false;
  }

  window.addEventListener('scroll', () => {
    if (!ticking) {
      requestAnimationFrame(updateParallax);
      ticking = true;
    }
  }, { passive: true });

  // ── Feature tabs ──
  const tabs = document.querySelectorAll('.feature-tab');
  const panels = document.querySelectorAll('.feature-panel');

  tabs.forEach((tab) => {
    tab.addEventListener('click', () => {
      const target = tab.dataset.tab;
      tabs.forEach((t) => t.classList.remove('active'));
      panels.forEach((p) => p.classList.remove('active'));
      tab.classList.add('active');
      document.querySelector(`[data-panel="${target}"]`).classList.add('active');
    });
  });

  // ── Hardware explode parallax ──
  const explodeSection = document.getElementById('explode-section');
  const explodeLayers = document.querySelectorAll('.explode-layer');

  function updateExplode() {
    if (!explodeSection) return;
    const rect = explodeSection.getBoundingClientRect();
    const progress = 1 - Math.max(0, Math.min(1, (rect.top + rect.height) / (window.innerHeight + rect.height)));
    explodeLayers.forEach((layer) => {
      const depth = parseInt(layer.dataset.depth, 10) || 1;
      const spread = progress * depth * 18;
      layer.style.transform = `translateY(${-spread}px)`;
    });
  }

  window.addEventListener('scroll', () => {
    requestAnimationFrame(updateExplode);
  }, { passive: true });

  // ── Hardware card tilt on mouse ──
  document.querySelectorAll('.hw-card').forEach((card) => {
    card.addEventListener('mousemove', (e) => {
      const rect = card.getBoundingClientRect();
      const x = (e.clientX - rect.left) / rect.width - 0.5;
      const y = (e.clientY - rect.top) / rect.height - 0.5;
      card.style.transform = `perspective(800px) rotateY(${x * 6}deg) rotateX(${-y * 6}deg)`;
    });
    card.addEventListener('mouseleave', () => {
      card.style.transform = '';
    });
  });

  // ── Three.js 3D Device ──
  const canvas = document.getElementById('device-canvas');
  if (canvas && typeof THREE !== 'undefined') {
    initDevice3D(canvas);
  }

  function initDevice3D(canvas) {
    const scene = new THREE.Scene();
    const renderer = new THREE.WebGLRenderer({ canvas, alpha: true, antialias: true });
    renderer.setPixelRatio(Math.min(window.devicePixelRatio, 2));
    renderer.shadowMap.enabled = true;
    renderer.shadowMap.type = THREE.PCFSoftShadowMap;

    const camera = new THREE.PerspectiveCamera(40, 1, 0.1, 100);
    camera.position.set(0, 0.5, 4.5);

    // Lighting
    const ambient = new THREE.AmbientLight(0xffffff, 0.6);
    scene.add(ambient);

    const keyLight = new THREE.DirectionalLight(0xffffff, 0.8);
    keyLight.position.set(3, 5, 4);
    keyLight.castShadow = true;
    scene.add(keyLight);

    const fillLight = new THREE.DirectionalLight(0xA8BEC9, 0.4);
    fillLight.position.set(-3, 2, -2);
    scene.add(fillLight);

    const rimLight = new THREE.DirectionalLight(0x7A99AB, 0.3);
    rimLight.position.set(0, -2, -3);
    scene.add(rimLight);

    // Device group
    const device = new THREE.Group();

    // Enclosure body
    const bodyGeo = new THREE.BoxGeometry(1.0, 2.2, 0.55);
    const bodyMat = new THREE.MeshStandardMaterial({
      color: 0xF4F4F2,
      roughness: 0.55,
      metalness: 0.05,
    });
    const body = new THREE.Mesh(bodyGeo, bodyMat);
    body.castShadow = true;
    body.receiveShadow = true;
    device.add(body);

    // Rounded edge simulation with smaller boxes
    const edgeMat = new THREE.MeshStandardMaterial({
      color: 0xECECEA,
      roughness: 0.6,
      metalness: 0.02,
    });

    // Three buttons
    const buttonPositions = [0.55, 0, -0.55];
    const buttons = [];

    buttonPositions.forEach((yPos) => {
      const btnGroup = new THREE.Group();

      const btnBase = new THREE.Mesh(
        new THREE.BoxGeometry(0.55, 0.55, 0.12),
        new THREE.MeshStandardMaterial({ color: 0xFAFAFA, roughness: 0.4, metalness: 0.1 })
      );
      btnBase.position.z = 0.32;
      btnBase.castShadow = true;
      btnGroup.add(btnBase);

      const btnIndent = new THREE.Mesh(
        new THREE.CylinderGeometry(0.1, 0.12, 0.04, 16),
        new THREE.MeshStandardMaterial({ color: 0xE8E8E6, roughness: 0.7 })
      );
      btnIndent.rotation.x = Math.PI / 2;
      btnIndent.position.set(0, 0, 0.39);
      btnGroup.add(btnIndent);

      btnGroup.position.y = yPos;
      device.add(btnGroup);
      buttons.push(btnGroup);
    });

    // USB-C port
    const usbGeo = new THREE.BoxGeometry(0.3, 0.12, 0.15);
    const usbMat = new THREE.MeshStandardMaterial({ color: 0x333333, roughness: 0.3, metalness: 0.8 });
    const usb = new THREE.Mesh(usbGeo, usbMat);
    usb.position.set(-0.55, 0.85, 0);
    device.add(usb);

    // PCB hint (visible gap)
    const pcbGeo = new THREE.BoxGeometry(0.85, 1.8, 0.04);
    const pcbMat = new THREE.MeshStandardMaterial({ color: 0x4A6741, roughness: 0.8 });
    const pcb = new THREE.Mesh(pcbGeo, pcbMat);
    pcb.position.z = -0.1;
    device.add(pcb);

    scene.add(device);

    // Ground plane for shadow
    const groundGeo = new THREE.PlaneGeometry(10, 10);
    const groundMat = new THREE.ShadowMaterial({ opacity: 0.08 });
    const ground = new THREE.Mesh(groundGeo, groundMat);
    ground.rotation.x = -Math.PI / 2;
    ground.position.y = -1.3;
    ground.receiveShadow = true;
    scene.add(ground);

    // Mouse interaction
    let targetRotX = 0;
    let targetRotY = 0;
    let currentRotX = 0;
    let currentRotY = 0;
    const hero3d = document.getElementById('hero-3d');

    if (hero3d) {
      hero3d.addEventListener('mousemove', (e) => {
        const rect = hero3d.getBoundingClientRect();
        const x = (e.clientX - rect.left) / rect.width - 0.5;
        const y = (e.clientY - rect.top) / rect.height - 0.5;
        targetRotY = x * 0.6;
        targetRotX = -y * 0.4;
      });

      hero3d.addEventListener('mouseleave', () => {
        targetRotX = 0;
        targetRotY = 0;
      });
    }

    // Scroll-based rotation
    function getScrollRotation() {
      const hero = document.getElementById('hero');
      if (!hero) return 0;
      const rect = hero.getBoundingClientRect();
      const progress = 1 - Math.max(0, Math.min(1, rect.bottom / window.innerHeight));
      return progress * 0.3;
    }

    // Resize
    function resize() {
      const parent = canvas.parentElement;
      const w = parent.clientWidth;
      const h = parent.clientHeight;
      renderer.setSize(w, h);
      camera.aspect = w / h;
      camera.updateProjectionMatrix();
    }

    resize();
    window.addEventListener('resize', resize);

    // Animate
    let time = 0;
    function animate() {
      requestAnimationFrame(animate);
      time += 0.01;

      currentRotX += (targetRotX - currentRotX) * 0.06;
      currentRotY += (targetRotY - currentRotY) * 0.06;

      const scrollRot = getScrollRotation();
      device.rotation.x = currentRotX + scrollRot * 0.5;
      device.rotation.y = currentRotY + Math.sin(time * 0.5) * 0.08;
      device.position.y = Math.sin(time * 0.8) * 0.04;

      // Button press animation
      buttons.forEach((btn, i) => {
        const press = Math.sin(time * 1.2 + i * 2.1) * 0.5 + 0.5;
        if (press > 0.95) {
          btn.position.z = -0.03;
        } else {
          btn.position.z = 0;
        }
      });

      renderer.render(scene, camera);
    }

    animate();
  }

  // ── Smooth anchor scroll offset for fixed nav ──
  document.querySelectorAll('a[href^="#"]').forEach((anchor) => {
    anchor.addEventListener('click', (e) => {
      const id = anchor.getAttribute('href');
      if (id === '#') return;
      const target = document.querySelector(id);
      if (!target) return;
      e.preventDefault();
      const offset = parseInt(getComputedStyle(document.documentElement).getPropertyValue('--nav-height'), 10) || 72;
      const top = target.getBoundingClientRect().top + window.scrollY - offset;
      window.scrollTo({ top, behavior: 'smooth' });
    });
  });
})();
