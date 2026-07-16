/* PROJECT OASIS — minimal chrome for workbench story */

(function () {
  'use strict';

  const nav = document.getElementById('nav');
  if (nav) {
    window.addEventListener('scroll', () => {
      nav.classList.toggle('scrolled', window.scrollY > 24);
    }, { passive: true });
  }
})();
