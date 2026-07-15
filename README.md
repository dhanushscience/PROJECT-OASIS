# PROJECT OASIS — Landing Page

Static one-page site for [PROJECT OASIS](https://github.com/dhanushscience/PROJECT-OASIS).

**Live URL:** https://dhanushscience.github.io/PROJECT-OASIS/

## GitHub Pages Setup

1. Open **Settings → Pages** in the GitHub repo
2. Under **Build and deployment**, set **Source** to **Deploy from a branch**
3. Select branch: `webpage` / folder: `/ (root)`
4. Save — the site will be live in 1–2 minutes

## Local Preview

```bash
# Python
python -m http.server 8080

# Node
npx serve .
```

Then open http://localhost:8080

## Structure

```
index.html          Main page
css/styles.css      OASIS theme styles
js/main.js          Parallax, 3D device, interactions
assets/images/      Product reference images
```

This branch contains **only** the marketing website. Project source code will be pushed to `master` separately.
