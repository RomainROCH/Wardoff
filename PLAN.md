## PLAN FINAL — Shutdown Blocker open-source en Rust

---

### 1. POSITIONNEMENT

**Tagline** : "Le premier outil open-source moderne pour reprendre le contrôle sur les shutdowns Windows."

**Licence** : MIT

**Public cible** (par priorité) :
1. Power users / gamers (streams interrompus, parties perdues)
2. Sysadmins (déploiement en entreprise, scripting, Event Log)
3. Développeurs (longs builds/tests, SSH sessions)
4. Communauté Rust (showcase d'un utilitaire système propre)

**Différenciation vs tous les concurrents** :
- Open-source (aucun concurrent ne l'est)
- Logs d'événements horodatés + intégration Windows Event Log (aucun)
- CLI complète pour scripting/automatisation (aucun sauf ShutdownBlocker basique)
- Gestion Windows 11 UpdateOrchestrator (tous ciblent MusNotification, obsolète)
- Transparence technique dans le README sur les 4 couches et leurs limites
- Distribution moderne : Winget, Scoop, Chocolatey
- Binaire natif unique ~1-3 Mo, zéro dépendance runtime

---

### 2. STACK TECHNIQUE

**Langage** : Rust, target `x86_64-pc-windows-msvc`

**Dépendances principales** :
- `windows` crate (Microsoft, v0.62+) — toutes les APIs Win32 : shutdown, tray, COM Task Scheduler, Event Log, ETW
- `tray-icon` ou `trayicon` — icône system tray cross-platform
- `ferrisetw` (v1.2, MIT/Apache-2.0, ~2.2K SLoC) — consumer ETW pour v1.0. Justification : utilisé en production par HarfangLab (EDR), thread-safety fixée en v1.0, code auditable car petit. Si un bug bloquant apparaît → fallback sur APIs ETW brutes via crate `windows`, ou fork+fix (le code est petit et lisible)
- `clap` — parsing CLI

---

### 3. ARCHITECTURE — LES 4 COUCHES DE SHUTDOWN

**Couche 1 — Shutdown standard (utilisateur/app)** *(fiable, API officielle)*
- Message-only window (`HWND_MESSAGE`) pour recevoir `WM_QUERYENDSESSION`
- `ShutdownBlockReasonCreate()` avec raison configurable par l'utilisateur
- Retourner `FALSE` sur `WM_QUERYENDSESSION`
- `SetProcessShutdownParameters(0x3FF, SHUTDOWN_NORETRY)` pour priorité maximale
- Résultat : Windows affiche l'écran "Cette app empêche l'arrêt"

**Couche 2 — shutdown.exe local** *(le problème dur)*
- **Mode standard (v1.0)** : ETW via `ferrisetw`, provider `Microsoft-Windows-Kernel-Process` (GUID `22fb2cd6-0e7b-422b-a0c7-2fad1fd0e716`), filtre sur ProcessStart event où ProcessName = `shutdown.exe`. Quand détecté → appel immédiat `AbortSystemShutdown()`. Fonctionne si timeout > 0. Pour `/t 0 /f` → log l'événement mais ne peut pas bloquer (documenté honnêtement).
- **Mode agressif opt-in (v1.0)** : IFEO — clé registry `HKLM\...\Image File Execution Options\shutdown.exe` avec Debugger pointant vers un proxy qui log + bloque. Warning clair dans l'UI et le README : "Peut déclencher des alertes EDR. Nettoyé proprement à la désactivation." Requiert admin.

**Couche 3 — Windows Update reboot** *(spécifique Win10/11)*
- API COM Task Scheduler (`ITaskService` → `ITaskFolder`) : surveiller et désactiver la tâche `Microsoft\Windows\UpdateOrchestrator\Reboot` tant que le mode Block est actif
- Polling toutes les 5 min pour re-désactiver si Windows la réactive
- ETW monitoring de `usoclient.exe` (même provider que couche 2)
- Requiert admin

**Couche 4 — Remote shutdown**
- Boucle polling `AbortSystemShutdown(NULL)` toutes les 900ms
- Fonctionne uniquement si le timeout distant est > 0
- Log la source quand détecté

**Bonus — Sleep/Hibernate/Screensaver** *(pour égaler Don't Sleep dès le MVP)*
- `SetThreadExecutionState(ES_CONTINUOUS | ES_SYSTEM_REQUIRED | ES_DISPLAY_REQUIRED)` — trivial, quelques lignes
- Toggle indépendant dans l'UI

---

### 4. FONCTIONNALITÉS

**UI** :
- Tray icon avec indicateur visuel Block (rouge) / Allow (vert)
- Menu clic-droit : Block / Allow / Shutdown / Reboot / Sleep / Hibernate / Settings / Quit
- Toast notifications Windows 11 natives quand un shutdown est bloqué
- Fenêtre Settings : profils, timer, mode agressif, options de log
- Possibilité de masquer complètement l'icône tray

**Profils** :
- **Gaming** : block shutdown + sleep + hibernate + screensaver + monitor off
- **Work** : block shutdown + sleep
- **Update Shield** : block uniquement les reboots Windows Update
- **Custom** : l'utilisateur choisit

**Timer** :
- "Bloquer pendant X minutes/heures, puis revenir en mode Allow"
- Countdown visible dans le tooltip de l'icône tray

**Logs** :
- Fichier texte rotatif (JSON lines) : timestamp, type d'événement, source, action prise, succès/échec
- Intégration Windows Event Log (EventLog provider personnalisé) — les sysadmins peuvent query via PowerShell/Event Viewer/SIEM
- Statistiques : compteur total, dernière tentative bloquée, source la plus fréquente

**CLI** :
```
app --block [--profile gaming|work|update|custom]
app --allow
app --status          (JSON: état actuel, compteurs, dernière action)
app --hide            (lance en arrière-plan sans UI)
app --log [--tail N]  (affiche les derniers événements)
app --aggressive on|off
```

**Auto-start** :
- Via Task Scheduler (pas la registry Run) pour garantir l'élévation admin si nécessaire
- Configurable dans Settings

---

### 5. ROADMAP

**MVP (lancement GitHub + premier post communautaire)** :
- Couche 1 complète (ShutdownBlockReasonCreate + WM_QUERYENDSESSION)
- Couche 3 complète (Task Scheduler UpdateOrchestrator)
- Couche 4 complète (boucle AbortSystemShutdown)
- Sleep/hibernate/screensaver blocking
- Tray icon Block/Allow + menu basique
- CLI : `--block`, `--allow`, `--status`, `--hide`
- Auto-start via Task Scheduler
- Log fichier texte simple
- README technique complet (4 couches, limites, comparatif)
- Pas d'ETW, pas de mode agressif — version "safe" pure APIs officielles

**v1.0 (post-feedback)** :
- Mode agressif opt-in (IFEO) avec warning
- ETW monitoring (ferrisetw) pour shutdown.exe/usoclient.exe + AbortSystemShutdown quand timeout > 0
- Windows Event Log integration
- Toast notifications Win11
- Timer
- Profils
- Distribution Winget + Scoop + Chocolatey

**v2.0 (si adoption significative)** :
- Triggers conditionnels (batterie < X%, CPU < X%, réseau idle)
- Dashboard/statistiques visuelles
- Driver kernel optionnel pour interception absolue de `/t 0 /f`
- Localisation i18n
- Remote management via named pipe ou CLI réseau

---

### 6. COMMUNICATION AU LANCEMENT

**Canaux** (dans cet ordre) :
1. GitHub — repo propre, README technique, badges, CI, releases avec binaire signé
2. r/rust — "I built a system tray shutdown blocker in Rust — the first open-source one"
3. r/sysadmin — angle Event Log + CLI + scripting + Windows Update shield
4. r/windows — angle "reprenez le contrôle, Don't Sleep est fermé et ne bloque pas shutdown.exe"
5. Hacker News — angle technique (les 4 couches, le dilemme IFEO, ETW)

**Messages clés** :
- "Le seul outil open-source dans cette catégorie — tous les autres sont closed-source ou abandonnés"
- "Honnête sur ses limites : shutdown /t 0 /f est inarrêtable en userspace, on le documente au lieu de le cacher"
- "Pensé pour les sysadmins : Event Log natif, CLI complète, déployable via Winget/GPO"
- "~2 Mo, zéro dépendance, binaire natif Rust"