# Maintainer Guide — GitHub Setup, Working with Claude, Going Public

## 1. Create the private repo (one time)

The local folder is already a git repo (`main` branch) with the docs.

```bash
# 1. Sign in to GitHub CLI (once per machine)
gh auth login

# 2. First commit
git add .
git commit -m "docs: initial PRD, architecture and roadmap"

# 3. Create the PRIVATE repo from this folder and push
gh repo create lunaRun --private --source=. --remote=origin --push \
  --description "Open-source, cross-platform multi-version PHP/Laravel local dev environment"
```

## 2. Repo settings (GitHub → Settings)

- **General:** enable *Automatically delete head branches*. Allow *Squash merging* only (keeps history clean for the changelog).
- **Actions → General:** *Workflow permissions* → "Read and write" (tauri-action needs it to create releases).
- **Secrets and variables → Actions:** add these once you reach the step that needs them:

| Secret | Needed from | How to get |
|---|---|---|
| `TAURI_SIGNING_PRIVATE_KEY` | M2.4 (can be added at M0.2) | `pnpm tauri signer generate -w ~/.tauri/lunarun.key` → file content |
| `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | same | the password you chose |
| `APPLE_CERTIFICATE`, `APPLE_CERTIFICATE_PASSWORD`, `APPLE_SIGNING_IDENTITY`, `APPLE_ID`, `APPLE_PASSWORD` (app-specific), `APPLE_TEAM_ID` | before public v0.1.0 | Apple Developer Program |

  **Back up the updater key and its password in a password manager.** If you lose it, auto-update breaks for every existing user.
- **Code security:** turn on Dependabot alerts + security updates and secret scanning (if your plan offers it for private repos).
- **Branch protection / rulesets** on `main` (require PR + green CI): private repos need a paid plan for this. On GitHub Free, add them when you go public.

## 3. Working with Claude (Opus for planning → Sonnet for building)

These docs are the contract. The usual loop:

1. Open the repo in VS Code with Claude Code (Sonnet).
2. Prompt: *"Read CLAUDE.md, then implement the next unchecked milestone in docs/ROADMAP.md on a new branch. Stop when the milestone's Definition of Done is met and summarise what changed."*
3. Review the diff, run the app (`pnpm tauri dev`), and merge the PR.
4. Repeat, one milestone at a time. Don't ask for a whole phase in one go.
5. When reality differs from the docs (an upstream URL changed, a crate doesn't work), Sonnet should **update the doc + add a DECISIONS entry**, not silently diverge. Switch back to Opus for big re-planning (new phase, a major design change).

## 4. Before going public — checklist

You can either **flip this repo to public** (Settings → Danger Zone → Change visibility), which keeps history, issues and releases, or push a fresh history to a new public repo. Flipping is simpler. Do it only if the history contains no secrets.

- [ ] Final name decided, trademark/domain check done (PRD Q1). Rename the repo/product if needed. GitHub redirects old URLs.
- [ ] `git log -p | grep -i -E "secret|password|token|api_key"` and a secret scanner (e.g. gitleaks) are clean
- [ ] README has screenshots, install steps, unsigned-app workarounds
- [ ] Add `CODE_OF_CONDUCT.md` (GitHub → Add file → choose the Contributor Covenant template)
- [ ] Enable **Private vulnerability reporting** (Settings → Code security) to match SECURITY.md
- [ ] Enable Discussions (optional), add topics: `php`, `laravel`, `xampp-alternative`, `local-development`, `tauri`
- [ ] Turn on branch protection/rulesets for `main`
- [ ] Publish the registry repo (`lunarun-registry`) publicly if not done yet
- [ ] macOS notarisation in place (PRD Q4)
- [ ] Tag `v0.1.0`, publish the release, announce

## 5. Why Apache-2.0 (short version)

Permissive like MIT, so anyone can use it, including companies, which helps adoption. It adds an explicit
patent grant and does **not** grant use of the lunaRun name/logo, so forks must rebrand.
See [DECISIONS D-007](DECISIONS.md#d-007--license-apache-20-2026-10-29-accepted). If you would rather force
forks to stay open source, the alternative is GPL-3.0. Decide **before** accepting outside contributions,
because relicensing later needs every contributor's permission.
