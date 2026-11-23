// The Settings pop-up's row icons, so a list of settings reads at a glance. Shared by Ico.svelte and legacy.js.

/** 24×24 stroke icons */
export const ICONS = {
  mic: 'M12 3a3 3 0 0 0-3 3v6a3 3 0 0 0 6 0V6a3 3 0 0 0-3-3zM6 11a6 6 0 0 0 12 0M12 17v4',
  sparkle: 'M12 3l1.8 5.2L19 10l-5.2 1.8L12 17l-1.8-5.2L5 10l5.2-1.8zM19 16l.7 2 2 .7-2 .7-.7 2-.7-2-2-.7 2-.7z',
  chip: 'M7 7h10v10H7zM10 10h4v4h-4zM9 3v4M15 3v4M9 17v4M15 17v4M3 9h4M3 15h4M17 9h4M17 15h4',
  press: 'M9 11V5a2 2 0 0 1 4 0v6M13 10V8a2 2 0 0 1 4 0v5a7 7 0 0 1-7 7 5 5 0 0 1-4.5-2.8L4 14a2 2 0 0 1 3.4-2L9 14',
  waves: 'M4 10v4M8 6v12M12 9v6M16 7v10M20 11v2',
  speaker: 'M4 9h4l5-4v14l-5-4H4zM17 9a4 4 0 0 1 0 6M19.5 6.5a8 8 0 0 1 0 11',
  screen: 'M3 5h18v11H3zM8 20h8M12 16v4',
  cloud: 'M7 18a4 4 0 0 1-.6-8A6 6 0 0 1 18 9a4.5 4.5 0 0 1-.5 9z',
  apikey: 'M14 4a6 6 0 1 1-5.2 9L3 18.8V21h3v-2h2v-2h2l.8-.8A6 6 0 0 1 14 4zM16 8h.01',
  refresh: 'M20 11a8 8 0 0 0-14.9-4M4 4v4h4M4 13a8 8 0 0 0 14.9 4M20 20v-4h-4',
  text: 'M4 7V5h16v2M12 5v14M9 19h6',
  branch: 'M6 4v6a4 4 0 0 0 4 4h8M14 10l4 4-4 4M6 20v-6',
  smile: 'M12 3a9 9 0 1 0 0 18 9 9 0 0 0 0-18zM8.5 14a4 4 0 0 0 7 0M9 9.5h.01M15 9.5h.01',
  list: 'M9 6h11M9 12h11M9 18h11M4 6h.01M4 12h.01M4 18h.01',
  mail: 'M3 6h18v12H3zM3 7l9 6 9-6',
  clock: 'M12 8v5l3 2M12 3a9 9 0 1 0 0 18 9 9 0 0 0 0-18z',
  shield: 'M12 3l7 3v5c0 5-3 8-7 10-4-2-7-5-7-10V6z',
  apps: 'M4 4h6v6H4zM14 4h6v6h-6zM4 14h6v6H4zM14 14h6v6h-6z',
  keyboard: 'M3 7h18v10H3zM7 11h.01M11 11h.01M15 11h.01M8 14h8',
  scissors: 'M6 9a3 3 0 1 0 0-6 3 3 0 0 0 0 6zM6 21a3 3 0 1 0 0-6 3 3 0 0 0 0 6zM8.1 7.9 20 20M8.1 16.1 20 4',
  globe: 'M12 3a9 9 0 1 0 0 18 9 9 0 0 0 0-18zM3 12h18M12 3a14 14 0 0 1 0 18M12 3a14 14 0 0 0 0 18',
  target: 'M12 3a9 9 0 1 0 0 18 9 9 0 0 0 0-18zM12 8a4 4 0 1 0 0 8 4 4 0 0 0 0-8zM12 12h.01',
  download: 'M12 4v11M7 10l5 5 5-5M5 20h14',
  trash: 'M4 7h16M10 11v6M14 11v6M6 7l1 13h10l1-13M9 7V4h6v3',
  up: 'M12 19V5M5 12l7-7 7 7',
  gauge: 'M4 18a8 8 0 1 1 16 0M12 18l4-6',
  layout: 'M4 4h16v16H4zM4 10h16M10 10v10',
  dock: 'M4 4h16v16H4zM15 4v16',
  eye: 'M2 12s4-7 10-7 10 7 10 7-4 7-10 7S2 12 2 12zM12 9a3 3 0 1 0 0 6 3 3 0 0 0 0-6z',
  palette: 'M12 3a9 9 0 0 0 0 18c1.1 0 1.5-.8 1.5-1.5 0-1.2-1-1.5-1-2.5s.8-2 2-2H17a4 4 0 0 0 4-4c0-4.4-4-8-9-8zM7.5 11h.01M10 7.5h.01M14.5 7.5h.01',
  sliders: 'M4 7h10M18 7h2M4 17h2M10 17h10M16 4v6M8 14v6',
  tag: 'M3 12V4h8l10 10-8 8zM7.5 8h.01',
  sun: 'M12 8a4 4 0 1 0 0 8 4 4 0 0 0 0-8zM12 2v2M12 20v2M4.9 4.9l1.4 1.4M17.7 17.7l1.4 1.4M2 12h2M20 12h2M4.9 19.1l1.4-1.4M17.7 6.3l1.4-1.4',
  power: 'M12 3v9M6.3 6.3a8 8 0 1 0 11.4 0',
  code: 'M8 8l-4 4 4 4M16 8l4 4-4 4M14 5l-4 14',
  compass: 'M12 3a9 9 0 1 0 0 18 9 9 0 0 0 0-18zM15.5 8.5l-2 5-5 2 2-5z',
  skip: 'M5 5l10 7-10 7zM19 5v14',
  fast: 'M3 6l8 6-8 6zM13 6l8 6-8 6z',
  link: 'M10 14a4 4 0 0 0 5.7 0l3-3a4 4 0 0 0-5.7-5.7l-1 1M14 10a4 4 0 0 0-5.7 0l-3 3a4 4 0 0 0 5.7 5.7l1-1',
  plug: 'M9 3v5M15 3v5M6 8h12v3a6 6 0 0 1-12 0zM12 17v4',
  calendar: 'M4 6h16v14H4zM4 10h16M8 3v5M16 3v5',
  chart: 'M4 20V10M10 20V4M16 20v-7M2 20h20',
  lock: 'M6 11h12v10H6zM8 11V7a4 4 0 0 1 8 0v4',
};

/** the icon for a setting the engine describes (legacy.js draws these rows) */
export const FIELD_ICONS = {
  'general.show_labels': 'tag', 'general.allow_scripts': 'code', 'general.bridge_enabled': 'link', 'general.bridge_port': 'plug',
  'youtube.auto_skip_ads': 'skip', 'youtube.gestures': 'press', 'youtube.hold2x_button': 'fast',
  'pad.show_desktop_button': 'screen', 'pad.show_desktop_hold_ms': 'clock',
  'meetings.reminders': 'calendar', 'meetings.remind_minutes': 'clock', 'meetings.join_minutes': 'clock',
};

/** an icon in a small tile, as a DOM node (legacy.js) */
export function icoNode(/** @type {string} */ name) {
  const span = document.createElement('span');
  span.className = 'ico';
  span.setAttribute('aria-hidden', 'true');
  span.innerHTML = `<svg viewBox="0 0 24 24"><path d="${ICONS[/** @type {keyof typeof ICONS} */ (name)] ?? ''}"/></svg>`;
  return span;
}

/** a group heading, as a DOM node (legacy.js); the same markup as Head.svelte */
export function headNode(/** @type {string} */ title, /** @type {string} */ text) {
  const head = document.createElement('div');
  head.className = 'ghead hero';
  const words = document.createElement('div');
  words.className = 'htext';
  const lab = document.createElement('span');
  lab.className = 'lab';
  lab.textContent = title;
  words.append(lab);
  if (text) {
    const p = document.createElement('p');
    p.className = 'gh';
    p.textContent = text;
    words.append(p);
  }
  head.append(words);
  return head;
}
