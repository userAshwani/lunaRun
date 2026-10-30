# Brand assets

The lunaRun logo is a crescent moon merged with an `R` and an arrow, with the wordmark `lunaRun`.

| File | Use |
|---|---|
| `source/logo-source.png` | **The one source of truth.** The original artwork cropped to the mark, the wordmark and the tagline on its flat navy background. |
| `logo-mark.png` | The mark alone on a transparent background, 1024 px square. |
| `logo-lockup-light.png` / `logo-lockup-dark.png` | Mark + `lunaRun` for README headers. Dark wordmark for light backgrounds, white wordmark for dark ones. |
| `app-icon-1024.png` | The mark on a navy rounded square. The input for the app icons. |
| `social-preview.png` | 1280 x 640 image. Upload it under GitHub > Settings > General > **Social preview** (this cannot be done from a commit). |

Everything except `source/` is **generated**. Do not edit the generated files by hand.

## Regenerating

```bash
pip install pillow numpy
python scripts/brand/make-brand-assets.py      # rebuilds the files above, src/assets/app-icon.png and public/favicon.png
pnpm tauri icon assets/app-icon-1024.png       # rebuilds src-tauri/icons/
```

`pnpm tauri icon` also creates `android/` and `ios/` folders and `64x64.png`, which lunaRun does not use
(mobile is a non-goal), so delete them before committing.

The script removes the navy background with a "color to alpha" step (only pixels brighter than the
background count, so there are no dark fringes on light backgrounds). To change the logo, replace
`source/logo-source.png` with new artwork on a flat background of the same navy (`#212235`), keeping the
mark, the wordmark and the tagline as three separate rows, then run the commands above.

Keep the original export outside the repository: the full-size file is about 5 MB and the export carries a
generator watermark in its bottom-right corner, which is cropped away in `source/`.
