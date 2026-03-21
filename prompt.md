# Prompt d'initialisation — Projet Wardoff

## Contexte

Tu travailles sur le repo Wardoff. Le fichier `.github/copilot-instructions.md` existe déjà et contient les conventions du projet. Le fichier `PLAN.md` est la source de vérité pour le scope, l'architecture et la roadmap. **Lis ces deux fichiers avant toute action.**

## Ta mission

Deux choses à faire dans cet ordre :

### Étape 1 — Renforcer copilot-instructions.md

Ajoute les sections suivantes à la fin du fichier `.github/copilot-instructions.md` existant. Ne modifie pas le contenu existant, ajoute uniquement après la dernière ligne.

```markdown

## Scope guardrails

These rules apply to EVERY change in this repo. Re-read them before starting any task.

### What is IN scope (MVP)
Only these features belong in the current phase:
- Layer 1: ShutdownBlockReasonCreate + WM_QUERYENDSESSION + SetProcessShutdownParameters
- Layer 3: Task Scheduler UpdateOrchestrator\Reboot disable/re-check
- Layer 4: AbortSystemShutdown polling loop
- Sleep/hibernate/screensaver blocking via SetThreadExecutionState
- Tray icon: Block/Allow toggle, right-click menu (Block, Allow, Shutdown, Reboot, Sleep, Hibernate, Quit)
- CLI: --block, --allow, --status (JSON output), --hide
- Auto-start via Task Scheduler
- File logging: rotating JSON lines (timestamp, event type, source, action, success/failure)
- README.md with technical documentation

### What is OUT of scope (v1.0 or later — do NOT implement)
- ETW monitoring (ferrisetw, Microsoft-Windows-Kernel-Process)
- IFEO aggressive mode
- Windows Event Log integration (EventLog provider)
- Toast notifications
- Timer functionality
- Profiles (Gaming, Work, Update Shield, Custom)
- Settings window / GUI beyond tray menu
- Winget/Scoop/Chocolatey packaging
- CI/CD pipelines, GitHub Actions workflows
- Release workflows, signing, packaging scripts
- Benchmarks, performance testing infrastructure

### Behavior rules for every commit
1. **No unrequested work.** Do not add features, files, configs, or infrastructure not explicitly asked for. No CI configs. No release workflows. No architectural refactoring beyond the current task.
2. **PLAN.md is the source of truth.** If a task contradicts PLAN.md, stop and ask. Do not silently deviate.
3. **One branch per fix/feature.** Create a branch named `feat/description` or `fix/description`, implement the change, then provide instructions to merge into the dev branch.
4. **No scope creep into v1.0 features.** If a task seems to require a v1.0 feature, flag it explicitly: "This requires [feature X] which is marked as v1.0 in PLAN.md. Should I proceed?"
5. **Document limitations honestly.** If something cannot be done (e.g., blocking `shutdown /t 0 /f`), document it in comments and README instead of implementing a hacky workaround.
6. **Admin boundaries are explicit.** Features requiring elevation must check for admin rights and fail with a clear message, not silently degrade.
7. **Test what you build.** After implementing a feature, compile it (`cargo build`) and verify it runs. If it requires Windows APIs that can only be tested at runtime, document what to test manually.

### Code conventions
- All public items must have `///` doc comments
- Use `log` crate macros (info!, warn!, error!) for all operational messages
- No `unwrap()` or `expect()` in production code paths — use proper error handling
- Module structure must match the layout defined in this file (see Module architecture below)
- Minimum Rust edition: 2021
- Target: x86_64-pc-windows-msvc only
```

### Étape 2 — Initialiser le projet Rust et créer la documentation

Après avoir mis à jour copilot-instructions.md, exécute ces tâches :

#### 2a. Initialiser le workspace Rust

Exécute `cargo init --name wardoff` dans le repo. Configure le `Cargo.toml` :

```toml
[package]
name = "wardoff"
version = "0.1.0"
edition = "2021"
authors = ["Romain ROCH"]
description = "Open-source Windows shutdown/reboot/sleep blocker"
license = "MIT"
repository = "https://github.com/Romain ROCH/wardoff"
keywords = ["windows", "shutdown", "blocker", "system", "tray"]
categories = ["os::windows-apis", "command-line-utilities"]
```

Pour les dépendances : vérifie les dernières versions stables sur crates.io avant de les ajouter. Les dépendances MVP sont :
- `windows` (features: Win32_System_Shutdown, Win32_UI_WindowsAndMessaging, Win32_System_Threading, Win32_Foundation, Win32_System_TaskScheduler, Win32_System_Com)
- `tray-icon`
- `clap` (features: derive)
- `log` + `env_logger`
- `serde` + `serde_json` (pour le log JSON et --status)
- `chrono` (timestamps)

Ajoute un `[profile.release]` optimisé pour la taille (opt-level "z", lto true, codegen-units 1, strip true).

#### 2b. Créer la structure de modules

```
src/
├── main.rs              (point d'entrée)
├── cli.rs               (définition clap)
├── blocker/
│   ├── mod.rs
│   ├── shutdown.rs      (couche 1)
│   ├── update.rs        (couche 3)
│   ├── remote.rs        (couche 4)
│   └── sleep.rs         (SetThreadExecutionState)
├── tray/
│   ├── mod.rs
│   └── icon.rs
├── logger/
│   └── mod.rs
└── config.rs
```

Chaque fichier .rs doit contenir :
- Les imports prévisibles
- Des structs/enums/traits vides avec des doc comments `///` décrivant leur rôle
- Des fonctions publiques avec `todo!("description de ce que cette fonction fera")` dans le body
- Pas de code fonctionnel — uniquement des squelettes documentés

#### 2c. Créer les fichiers projet

- `LICENSE` — MIT, année 2026, auteur Romain ROCH
- `CHANGELOG.md` — vide avec un header template (## [Unreleased])
- `CONTRIBUTING.md` — comment builder (prérequis Rust + MSVC Build Tools), comment tester (Windows 10+ ou VM), conventions (rustfmt, clippy), process de PR
- `.gitignore` — Rust standard (target/, *.pdb, etc.)

#### 2d. Créer la documentation technique dans docs/

- `docs/WINDOWS_SHUTDOWN_LAYERS.md` — document approfondi sur les 4 couches. Toutes les informations techniques sont dans PLAN.md sections 3. Reformule-les en anglais technique accessible, avec des exemples d'API calls et les GUIDs/noms de tâches exacts.

- `docs/COMPARISON.md` — tableau comparatif détaillé :
  - ShutdownBlocker (cresstone.com) : .NET 4.0, freeware fermé, dernière MàJ mars 2017, utilise IFEO pour shutdown.exe
  - ShutdownGuard (stefansundin/shutdownguard) : C pur MinGW, MIT, archivé "UNSUPPORTED" depuis 2014, utilisait DLL injection
  - Don't Sleep (softwareok.com) : freeware fermé (reverse-engineering interdit), activement maintenu, ne bloque PAS shutdown.exe, pas d'Event Log
  - PreventTurnOff (softwareok.com) : version simplifiée de Don't Sleep, mêmes limitations
  Pour chaque concurrent : stack, mécanismes, ce qui manque, statut

- `docs/IFEO_WARNING.md` — documentation du futur mode agressif (v1.0). Expliquer IFEO, pourquoi c'est nécessaire (seule méthode contre /t 0 /f), pourquoi c'est controversé (MITRE ATT&CK T1546.012, détecté par EDR), que ce sera opt-in avec nettoyage propre. Marquer clairement "PLANNED — NOT YET IMPLEMENTED".

#### 2e. Créer le README.md

Le README doit contenir, dans cet ordre :
1. Nom + tagline + badges (MIT, Rust, Windows)
2. Paragraphe d'accroche : le problème + pourquoi les alternatives ne suffisent pas
3. Features MVP (liste de ce qui est implémenté)
4. Planned features v1.0 (liste marquée "planned")
5. Honest limitations (shutdown /t 0 /f, admin requis pour certaines features)
6. How it works (lien vers docs/WINDOWS_SHUTDOWN_LAYERS.md)
7. Installation (placeholder : cargo install, GitHub Releases)
8. Usage (exemples CLI + description du tray)
9. Comparison (tableau résumé, lien vers docs/COMPARISON.md)
10. Contributing (lien vers CONTRIBUTING.md)
11. License (MIT)

Tout en anglais. Ton technique mais accessible.

## Consignes finales

- Tous les documents en anglais
- Ne crée AUCUN fichier qui n'est pas listé ci-dessus (pas de .github/workflows, pas de Dockerfile, pas de scripts)
- Après avoir tout créé, lance `cargo check` pour vérifier que le projet compile (même si c'est que des todo!())
- Résume ce que tu as fait et liste les fichiers créés