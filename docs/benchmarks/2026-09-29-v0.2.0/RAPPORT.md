# Benchmark Wardoff 0.2.0


## Comparaison des moyennes

| Mesure | Allow | Block |
|---|---:|---:|
| Mémoire résidente (MiB) | 12.57 | 13.95 |
| Mémoire privée (MiB) | 1.96 | 3.25 |
| CPU (% du total machine) | 0.0024 | 0.0050 |

**Allow** : protections désactivées. **Block** : protections actives en arrière-plan. Les valeurs CPU sont des pourcentages de la capacité totale de la machine.

Mesures locales commencées le 2026-09-29 (UTC). Binaire officiel de la dernière release disponible au moment de cette campagne, vérifié avant exécution.

## Résultats

| Scénario | Mesure | Moyenne | Médiane | P95 | Maximum |
|---|---|---:|---:|---:|---:|
| Allow | Mémoire privée (MiB) | 1.955 | 1.957 | 2.020 | 2.043 |
| Allow | Working Set (MiB) | 12.568 | 12.562 | 12.715 | 12.754 |
| Allow | CPU Wardoff (% du total machine) | 0.00239 | 0.00000 | 0.00000 | 0.19715 |
| Allow | Threads | 5.986 | 5.000 | 8.000 | 8.000 |
| Block | Mémoire privée (MiB) | 3.251 | 3.246 | 3.336 | 3.359 |
| Block | Working Set (MiB) | 13.946 | 14.047 | 14.266 | 14.277 |
| Block | CPU Wardoff (% du total machine) | 0.00499 | 0.00000 | 0.00000 | 0.19705 |
| Block | Threads | 10.333 | 10.000 | 13.000 | 13.000 |

Les moyennes CPU sont pondérées par la durée réelle des intervalles. Médiane, P95 et maximum décrivent les échantillons d’environ une seconde. Un maximum ici ne représente pas un pic instantané.

## Répétitions et activité ambiante

| Répétition | Scénario | CPU Wardoff moyen (%) | Working Set moyen (MiB) | CPU système moyen (%) |
|---|---|---:|---:|---:|
| 1 | baseline | — | — | 9.427 |
| 1 | allow | 0.00260 | 12.527 | 9.332 |
| 1 | block | 0.00391 | 14.069 | 8.863 |
| 2 | baseline | — | — | 9.093 |
| 2 | allow | 0.00326 | 12.607 | 9.528 |
| 2 | block | 0.00846 | 13.670 | 9.669 |
| 3 | baseline | — | — | 9.412 |
| 3 | allow | 0.00130 | 12.568 | 8.561 |
| 3 | block | 0.00260 | 14.100 | 8.549 |

La baseline sert à documenter l’activité ambiante. Elle n’est pas soustraite aux mesures du processus. Le CPU système inclut toutes les applications, dont le collecteur et l’environnement de travail ; il ne mesure pas le coût causal de Wardoff.

## Protocole

- 3 répétitions × 3 scénarios × 300 secondes : 45 minutes mesurées, 900 échantillons par scénario.
- Baseline : aucun processus Wardoff. Allow : icône visible, protections désactivées. Block : même interface visible, protections en fonctionnement normal, sans tentative réelle d’arrêt.
- Ordres : baseline/allow/block, allow/block/baseline, block/baseline/allow. Nouveau processus pour chaque phase avec Wardoff ; stabilisation de 30 secondes exclue avant chaque mesure.
- Échantillonnage sur horloge monotone, cible d’une seconde. CPU = différence de temps processeur / temps réel / 8 processeurs logiques × 100. 100 % correspond à toute la machine ; multiplier par 8 pour la convention où un cœur logique vaut 100 %.
- Mémoire privée : PrivateMemorySize64, engagement privé du processus, pas nécessairement résident. Working Set : mémoire résidente du processus, pages partagées incluses. Ce ne sont pas deux mesures à additionner. 1 MiB = 1 048 576 octets.
- P95 : rang le plus proche par excès (ceil(0,95 × N)). Statistiques descriptives ; les secondes consécutives ne sont pas des répétitions indépendantes.
- État et couches contrôlés hors mesure au début et à la fin ; sortie normale via WM_QUIT, avec attente de fermeture. Aucun arrêt Windows, redémarrage, suspension, changement de démarrage automatique ou fermeture d’applications tierces.
- Le collecteur empêche la veille système de manière identique dans tous les scénarios ; le plan d’alimentation reste inchangé. Données conservées en mémoire pendant chaque phase, exportées entre phases.

## Machine et provenance

- CPU : Intel(R) Core(TM) i7-6700HQ CPU @ 2.60GHz, 4 cœurs / 8 processeurs logiques.
- RAM visible par Windows : 15.867 GiB (machine équipée de 16 Go).
- Système : Microsoft Windows 11 IoT Enterprise LTSC, 10.0.26100, build 26100.9445 (révision vérifiée après mesure).
- Alimentation : Power Scheme GUID: 381b4222-f694-41f0-9685-ff5bb260df2e  (Balanced). Exécution administrateur : True.
- Release : [v0.2.0](https://github.com/RomainROCH/Wardoff/releases/tag/v0.2.0). Version affichée par le binaire : wardoff 0.2.0.
- SHA-256 du binaire : `96d1700c492b1689e6cb3a18847481eb9a203e30db4d4ba273d4b458ed6a84bc` ; correspond au digest de l’asset GitHub.
- Commit du tag : `9f6c0375c851d0a5b933da0bd69bafc75b215cf5`. HEAD local/main distant vérifié : `aa40e8982273fa66b86b806ec1424a547affdbd3`.
- Après récupération des références distantes, aucune différence de code applicatif, manifeste, script de build ou dépendances entre le tag, main et dev. Les différences concernent la documentation et les consignes.
- Tentative de reconstruction avec cargo build --release --locked refusée par Cargo car le verrou des dépendances nécessite une mise à jour. Les mesures concernent donc explicitement le binaire distribué, pas une reconstruction locale. Le rattachement à la release et son digest sont vérifiés ; aucune reproductibilité binaire n’est revendiquée.

## Couches réellement actives

- `shutdown` : active.
- `local_shutdown` : active.
- `update` : inactive.
- `remote` : active.
- `sleep` : active.

Le journal du runtime administrateur confirme que la tâche `Microsoft\Windows\UpdateOrchestrator\Reboot` est absente sur cette machine LTSC. La couche Update est donc ignorée normalement. Ces résultats couvrent seulement cette combinaison effective de couches. La surveillance locale signalée active par le runtime ne constitue pas une preuve d’efficacité contre un arrêt forcé.

## Contrôles et limites

- Validation : 2700 échantillons, 9 phases complètes, état et identité du processus vérifiés.
- Intervalles > 1,25 s : 0 ; intervalle maximal : 1.025 s.
- Le CPU du collecteur figure dans summary.csv et dans les données brutes. Il reste inclus dans le CPU système.
- CPU système pendant les baselines : moyenne 9.31 %, P95 17.12 %, maximum 86.92 %. Le bureau n’était donc pas au repos absolu. Aucun échantillon n’a été supprimé.
- CPU moyen du collecteur : baseline 0.014 %, Allow 0.049 %, Block 0.040 % du total machine. Son coût reste distinct du processus Wardoff.
- Les moyennes CPU et Working Set ont été recalculées indépendamment en PowerShell à partir des CSV ; elles concordent avec les résultats Python.
- Le processus CPU peut être quantifié par le compteur Windows : un échantillon à zéro ne prouve pas une absence absolue de travail. La moyenne intégrée sur 15 minutes par mode est plus informative.
- Mesure sur une machine utilisée normalement, avec ses services et applications ouverts, pas sur une image Windows isolée. Pas de contrôle des températures, fréquences instantanées ou activité utilisateur. Ne pas attribuer une différence de CPU système à Wardoff.
- Démarrage exclu ; pas de test de charge de commandes, de vrai blocage d’arrêt, de batterie ou de fuite mémoire sur plusieurs heures. Résultats non généralisables à toutes les machines ou toutes les protections.
- Les compteurs I/O sont ceux du processus (fichiers, périphériques, réseau), pas une mesure du trafic physique du disque.

## Fichiers et reproduction

- `Measure-Wardoff.ps1` : collecteur autonome PowerShell ; lancer en administrateur avec le binaire vérifié dans le même dossier.
- `results/` : CSV bruts, états JSON, métadonnées, journal des phases et statut final.
- `analyze.py` : validation et calcul des statistiques avec Python standard, sans dépendances supplémentaires.
- `summary.csv`, `summary.json`, `validation.json` : résultats et contrôles reproductibles.
- Le pilote technique court est exclu des statistiques et de ce dossier de preuves ; il reste dans l’archive locale initiale.
- L’empreinte du collecteur n’a pas été enregistrée au démarrage de la campagne. Le script conservé est identique à la copie de l’archive initiale ; le manifeste ultérieur protège cette copie sans constituer une attestation cryptographique de son exécution.

Ce dossier conserve les preuves de la campagne. Son état de sauvegarde distante doit être vérifié par la référence Git distante ; la présence de ces fichiers ne constitue pas à elle seule une sauvegarde hors machine.

## Formulation utilisable avec son contexte

> On an Intel Core i7-6700HQ running Windows 11 IoT Enterprise LTSC, Wardoff 0.2.0 averaged approximately 14 MiB resident memory and less than 0.01% total-system CPU in Block mode across three five-minute background runs. UpdateOrchestrator task protection was unavailable because the task was absent.

Cette formulation décrit une mesure du processus sur cette machine ; elle ne promet ni la même consommation partout, ni une protection contre tous les arrêts Windows.
