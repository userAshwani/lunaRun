# Brand assets

- `logo.svg` is the source of truth for the lunaRun logo (an indigo rounded square with a white crescent moon).
- `logo-1024.png` is `logo.svg` rendered at 1024 x 1024 with a transparent background. It is the input for the app icons.

To change the logo, edit `logo.svg`, re-render `logo-1024.png` (any SVG renderer), then regenerate the icons:

```bash
pnpm tauri icon assets/logo-1024.png
```

That rewrites `src-tauri/icons/`. It also creates `android/` and `ios/` folders and `64x64.png`, which lunaRun does not use
(mobile is a non-goal), so delete them before committing.
