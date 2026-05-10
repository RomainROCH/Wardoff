# --- CLAUDE.md pour Wardoff (racine du repo) ---

# Projet

Application Rust Windows : bloqueur de shutdown en 4 couches.
MVP tagué `v0.1.0-mvp`. Repo public, licence MIT.

# Build et vérification

```
cargo build --release
cargo test
cargo clippy -- -D warnings
```

Exécute les trois après chaque changement. Binary cible < 2MB, zéro runtime deps.

# Architecture

4 couches de blocage shutdown — consulter `src/` directement.
Architecture détaillée : @docs/ARCHITECTURE.md
Business model : @docs/BUSINESS_MODEL.md

# Tests

`cargo test` couvre les tests unitaires et tourne sans privilèges.
Les couches 3 (Update Orchestrator) et 4 (remote shutdown) ne s'arment qu'en session élevée — leur validation est manuelle.
Procédure complète et matrice : @docs/MANUAL_VALIDATION_PLAN.md

# Git

- Branche principale : `main`
- Tags sémantiques : `v0.x.y`
- Commits atomiques
- Format Conventional Commits : `type: sujet` en minuscules. Types utilisés : `feat`, `fix`, `docs`, `chore`, `refactor`, `test`, `merge`. Sujet à l'impératif présent, sans point final.

# Ne PAS faire

- Pas de portabilité cross-platform — Windows only, c'est intentionnel
- Pas de telemetry, pas de paywall, pas de tracking
- Ne pas ajouter de dépendances sans demander
- Ne pas utiliser `unwrap()` en production
