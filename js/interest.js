/**
 * End CTA: drive GitHub stars as the interest signal.
 * Star count is read from the public GitHub API when available.
 */

const REPO = 'dhanushscience/PROJECT-OASIS';
const REPO_URL = `https://github.com/${REPO}`;

function setNote(text, kind = '') {
  const note = document.getElementById('interest-note');
  if (!note) return;
  note.textContent = text;
  note.dataset.kind = kind;
}

function updateStarCount(n) {
  const el = document.getElementById('interest-count');
  if (!el) return;
  if (!Number.isFinite(n) || n < 1) {
    el.hidden = true;
    return;
  }
  el.hidden = false;
  el.textContent = `${n} GitHub star${n === 1 ? '' : 's'} so far`;
}

async function fetchStarCount() {
  try {
    const res = await fetch(`https://api.github.com/repos/${REPO}`, {
      headers: { Accept: 'application/vnd.github+json' },
    });
    if (!res.ok) return null;
    const data = await res.json();
    const n = Number(data.stargazers_count);
    return Number.isFinite(n) ? n : null;
  } catch {
    return null;
  }
}

function initStarCta() {
  const btn = document.getElementById('end-star-btn');
  if (btn) {
    btn.href = REPO_URL;
    btn.addEventListener('click', () => {
      setNote('Thanks. Star the repo so you are first in line at launch.', 'ok');
      // Refresh count after they return from GitHub
      window.setTimeout(() => {
        fetchStarCount().then((n) => {
          if (n != null) updateStarCount(n);
        });
      }, 4000);
    });
  }

  fetchStarCount().then((n) => {
    if (n != null) updateStarCount(n);
  });
}

initStarCta();
