"""Validate all samples and compute reproducible nearest-rank descriptive statistics."""
import csv, json, math, statistics
from pathlib import Path

root = Path(__file__).resolve().parent
folder = root / 'results'
meta = json.loads((folder / 'metadata.json').read_text(encoding='utf-8-sig'))
provenance = json.loads((root/'provenance.json').read_text(encoding='utf-8-sig')) if (root/'provenance.json').exists() else {}
assert (folder / 'status.txt').read_text(encoding='utf-8-sig').strip() == 'SUCCESS'
metrics = ['cpu_pct','private_mib','working_set_mib','threads','handles',
           'io_read_bytes_s','io_write_bytes_s','system_cpu_pct','collector_cpu_pct','interval_s']
all_rows = []
checks = []
layer_sets = {}
for run in range(1, meta['repeats'] + 1):
    for scenario in ['baseline','allow','block']:
        label = f'run-{run}-{scenario}'
        with (folder / f'{label}.csv').open(encoding='utf-8-sig', newline='') as stream:
            rows = list(csv.DictReader(stream))
        assert len(rows) == meta['seconds'], (label, len(rows))
        assert [int(r['sample']) for r in rows] == list(range(1,len(rows)+1))
        assert all(r['scenario']==scenario and int(r['run'])==run for r in rows)
        for r in rows:
            for key in metrics + ['elapsed_s','process_cpu_s']:
                r[key] = float(r[key]) if r[key] else None
                assert r[key] is None or math.isfinite(r[key])
            assert 0 < r['interval_s'] <= 1.25, (label, 'Sampling interval outside campaign tolerance')
            assert 0 <= r['system_cpu_pct'] <= 100
        assert all(rows[i]['elapsed_s'] > rows[i-1]['elapsed_s'] for i in range(1,len(rows)))
        assert abs(rows[-1]['elapsed_s'] - meta['seconds']) <= 1.25, (label, 'Incorrect phase duration')
        # The stopwatch starts before the initial counter snapshot; retain that offset.
        initial_offset = rows[0]['elapsed_s'] - rows[0]['interval_s']
        assert 0 <= initial_offset < 0.1, (label, 'Unexpected initial timing offset')
        assert abs(sum(r['interval_s'] for r in rows) - (rows[-1]['elapsed_s'] - initial_offset)) < 0.000001, (label, 'Inconsistent elapsed time')
        if scenario != 'baseline':
            assert len({r['pid'] for r in rows}) == 1
            assert all(0 <= r['cpu_pct'] <= 100 and r['private_mib'] > 0 for r in rows)
            start=json.loads((folder/f'{label}-start.json').read_text(encoding='utf-8-sig'))
            end=json.loads((folder/f'{label}-end.json').read_text(encoding='utf-8-sig'))
            assert start['state'] == end['state'] == scenario
            assert start['layers'] == end['layers'], label
            assert start['layers'] == layer_sets.setdefault(scenario, start['layers']), label
            assert end['blocked_count'] == start['blocked_count'], (label, 'Unexpected shutdown activity')
            assert end['uptime_seconds'] >= start['uptime_seconds'] + meta['seconds'] - 2
        checks.append({'run':run,'scenario':scenario,'samples':len(rows),
                       'elapsed_s':rows[-1]['elapsed_s'],
                       'intervals_over_1_25s':sum(r['interval_s']>1.25 for r in rows),
                       'max_interval_s':max(r['interval_s'] for r in rows)})
        all_rows.extend(rows)

summaries=[]
for scope in ['pooled', *range(1, meta['repeats']+1)]:
    for scenario in ['baseline','allow','block']:
        rows=[r for r in all_rows if r['scenario']==scenario and (scope=='pooled' or int(r['run'])==scope)]
        for metric in metrics:
            values=sorted(r[metric] for r in rows if r[metric] is not None)
            if not values: continue
            mean=statistics.mean(values)
            if metric in ['cpu_pct','collector_cpu_pct','system_cpu_pct','io_read_bytes_s','io_write_bytes_s']:
                mean=sum(r[metric]*r['interval_s'] for r in rows)/sum(r['interval_s'] for r in rows)
            summaries.append(dict(scope=scope,scenario=scenario,metric=metric,n=len(values),mean=mean,
                                  median=statistics.median(values),p95=values[math.ceil(.95*len(values))-1],
                                  maximum=max(values),minimum=min(values)))
with (root/'summary.csv').open('w',encoding='utf-8',newline='') as stream:
    writer=csv.DictWriter(stream,fieldnames=list(summaries[0]));writer.writeheader();writer.writerows(summaries)
(root/'validation.json').write_text(json.dumps({'passed':True,'samples':len(all_rows),'runs':checks},indent=2))
(root/'summary.json').write_text(json.dumps(summaries,indent=2))
def stat(scenario, metric, scope='pooled'):
    return next(s for s in summaries if s['scope']==scope and s['scenario']==scenario and s['metric']==metric)

comparison = ['', '## Comparaison des moyennes', '',
              '| Mesure | Allow | Block |', '|---|---:|---:|']
for metric, label, digits in [('working_set_mib', 'Mémoire résidente (MiB)', 2),
                              ('private_mib', 'Mémoire privée (MiB)', 2),
                              ('cpu_pct', 'CPU (% du total machine)', 4)]:
    comparison.append(f"| {label} | {stat('allow', metric)['mean']:.{digits}f} | {stat('block', metric)['mean']:.{digits}f} |")
comparison += ['', '**Allow** : protections désactivées. **Block** : protections actives en arrière-plan. Les valeurs CPU sont des pourcentages de la capacité totale de la machine.', '']
lines = ['# Benchmark Wardoff 0.2.0', '', *comparison,
         f"Mesures locales commencées le {meta['startedUtc'][:10]} (UTC). Binaire officiel de la dernière release disponible au moment de cette campagne, vérifié avant exécution.", '',
         '## Résultats', '',
         '| Scénario | Mesure | Moyenne | Médiane | P95 | Maximum |',
         '|---|---|---:|---:|---:|---:|']
for scenario in ['allow','block']:
    for metric, label in [('private_mib','Mémoire privée (MiB)'),('working_set_mib','Working Set (MiB)'),('cpu_pct','CPU Wardoff (% du total machine)'),('threads','Threads')]:
        s=stat(scenario,metric)
        digits=5 if metric=='cpu_pct' else 3
        vals=' | '.join(f'{s[k]:.{digits}f}' for k in ['mean','median','p95','maximum'])
        lines.append(f'| {scenario.title()} | {label} | {vals} |')
lines += ['', 'Les moyennes CPU sont pondérées par la durée réelle des intervalles. Médiane, P95 et maximum décrivent les échantillons d’environ une seconde. Un maximum ici ne représente pas un pic instantané.', '',
          '## Répétitions et activité ambiante', '',
          '| Répétition | Scénario | CPU Wardoff moyen (%) | Working Set moyen (MiB) | CPU système moyen (%) |',
          '|---|---|---:|---:|---:|']
for run in range(1,meta['repeats']+1):
    for scenario in ['baseline','allow','block']:
        cpu='—' if scenario=='baseline' else f"{stat(scenario,'cpu_pct',run)['mean']:.5f}"
        ram='—' if scenario=='baseline' else f"{stat(scenario,'working_set_mib',run)['mean']:.3f}"
        system=stat(scenario,'system_cpu_pct',run)['mean']
        lines.append(f'| {run} | {scenario} | {cpu} | {ram} | {system:.3f} |')
lines += ['', 'La baseline sert à documenter l’activité ambiante. Elle n’est pas soustraite aux mesures du processus. Le CPU système inclut toutes les applications, dont le collecteur et l’environnement de travail ; il ne mesure pas le coût causal de Wardoff.', '',
          '## Protocole', '',
          f"- {meta['repeats']} répétitions × 3 scénarios × {meta['seconds']} secondes : {meta['repeats']*3*meta['seconds']/60:g} minutes mesurées, {meta['repeats']*meta['seconds']} échantillons par scénario.",
          '- Baseline : aucun processus Wardoff. Allow : icône visible, protections désactivées. Block : même interface visible, protections en fonctionnement normal, sans tentative réelle d’arrêt.',
          '- Ordres : baseline/allow/block, allow/block/baseline, block/baseline/allow. Nouveau processus pour chaque phase avec Wardoff ; stabilisation de 30 secondes exclue avant chaque mesure.',
          f"- Échantillonnage sur horloge monotone, cible d’une seconde. CPU = différence de temps processeur / temps réel / {meta['logicalProcessors']} processeurs logiques × 100. 100 % correspond à toute la machine ; multiplier par {meta['logicalProcessors']} pour la convention où un cœur logique vaut 100 %.",
          '- Mémoire privée : PrivateMemorySize64, engagement privé du processus, pas nécessairement résident. Working Set : mémoire résidente du processus, pages partagées incluses. Ce ne sont pas deux mesures à additionner. 1 MiB = 1 048 576 octets.',
          '- P95 : rang le plus proche par excès (ceil(0,95 × N)). Statistiques descriptives ; les secondes consécutives ne sont pas des répétitions indépendantes.',
          '- État et couches contrôlés hors mesure au début et à la fin ; sortie normale via WM_QUIT, avec attente de fermeture. Aucun arrêt Windows, redémarrage, suspension, changement de démarrage automatique ou fermeture d’applications tierces.',
          '- Le collecteur empêche la veille système de manière identique dans tous les scénarios ; le plan d’alimentation reste inchangé. Données conservées en mémoire pendant chaque phase, exportées entre phases.', '',
          '## Machine et provenance', '',
          f"- CPU : {meta['cpu'][0]['Name']}, {sum(c['NumberOfCores'] for c in meta['cpu'])} cœurs / {meta['logicalProcessors']} processeurs logiques.",
          f"- RAM visible par Windows : {meta['os']['TotalVisibleMemorySize']/1024/1024:.3f} GiB (machine équipée de 16 Go).",
          f"- Système : {meta['os']['Caption']}, {meta['os']['Version']}, build {provenance.get('osBuild',meta['os']['BuildNumber'])} (révision vérifiée après mesure).",
          f"- Alimentation : {meta['powerPlan']}. Exécution administrateur : {meta['admin']}.",
          '- Release : [v0.2.0](https://github.com/RomainROCH/Wardoff/releases/tag/v0.2.0). Version affichée par le binaire : wardoff 0.2.0.',
          f"- SHA-256 du binaire : `{meta['binarySha256'].lower()}` ; correspond au digest de l’asset GitHub.",
          f"- Commit du tag : `{meta['releaseCommit']}`. HEAD local/main distant vérifié : `{meta['mainCommit']}`.",
          '- Après récupération des références distantes, aucune différence de code applicatif, manifeste, script de build ou dépendances entre le tag, main et dev. Les différences concernent la documentation et les consignes.',
          '- Tentative de reconstruction avec cargo build --release --locked refusée par Cargo car le verrou des dépendances nécessite une mise à jour. Les mesures concernent donc explicitement le binaire distribué, pas une reconstruction locale. Le rattachement à la release et son digest sont vérifiés ; aucune reproductibilité binaire n’est revendiquée.', '',
          '## Couches réellement actives', '']
block_status=json.loads((folder/'run-1-block-start.json').read_text(encoding='utf-8-sig'))
for layer, active in block_status['layers'].items():
    lines.append(f"- `{layer}` : {'active' if active else 'inactive'}.")
lines += ['', 'Le journal du runtime administrateur confirme que la tâche `Microsoft\\Windows\\UpdateOrchestrator\\Reboot` est absente sur cette machine LTSC. La couche Update est donc ignorée normalement. Ces résultats couvrent seulement cette combinaison effective de couches. La surveillance locale signalée active par le runtime ne constitue pas une preuve d’efficacité contre un arrêt forcé.', '',
          '## Contrôles et limites', '',
          f"- Validation : {len(all_rows)} échantillons, {len(checks)} phases complètes, état et identité du processus vérifiés.",
          f"- Intervalles > 1,25 s : {sum(c['intervals_over_1_25s'] for c in checks)} ; intervalle maximal : {max(c['max_interval_s'] for c in checks):.3f} s.",
          '- Le CPU du collecteur figure dans summary.csv et dans les données brutes. Il reste inclus dans le CPU système.',
          f"- CPU système pendant les baselines : moyenne {stat('baseline','system_cpu_pct')['mean']:.2f} %, P95 {stat('baseline','system_cpu_pct')['p95']:.2f} %, maximum {stat('baseline','system_cpu_pct')['maximum']:.2f} %. Le bureau n’était donc pas au repos absolu. Aucun échantillon n’a été supprimé.",
          f"- CPU moyen du collecteur : baseline {stat('baseline','collector_cpu_pct')['mean']:.3f} %, Allow {stat('allow','collector_cpu_pct')['mean']:.3f} %, Block {stat('block','collector_cpu_pct')['mean']:.3f} % du total machine. Son coût reste distinct du processus Wardoff.",
          '- Les moyennes CPU et Working Set ont été recalculées indépendamment en PowerShell à partir des CSV ; elles concordent avec les résultats Python.',
          '- Le processus CPU peut être quantifié par le compteur Windows : un échantillon à zéro ne prouve pas une absence absolue de travail. La moyenne intégrée sur 15 minutes par mode est plus informative.',
          '- Mesure sur une machine utilisée normalement, avec ses services et applications ouverts, pas sur une image Windows isolée. Pas de contrôle des températures, fréquences instantanées ou activité utilisateur. Ne pas attribuer une différence de CPU système à Wardoff.',
          '- Démarrage exclu ; pas de test de charge de commandes, de vrai blocage d’arrêt, de batterie ou de fuite mémoire sur plusieurs heures. Résultats non généralisables à toutes les machines ou toutes les protections.',
          '- Les compteurs I/O sont ceux du processus (fichiers, périphériques, réseau), pas une mesure du trafic physique du disque.', '',
          '## Fichiers et reproduction', '',
          '- `Measure-Wardoff.ps1` : collecteur autonome PowerShell ; lancer en administrateur avec le binaire vérifié dans le même dossier.',
          '- `results/` : CSV bruts, états JSON, métadonnées, journal des phases et statut final.',
          '- `analyze.py` : validation et calcul des statistiques avec Python standard, sans dépendances supplémentaires.',
          '- `summary.csv`, `summary.json`, `validation.json` : résultats et contrôles reproductibles.',
          '- Le pilote technique court est exclu des statistiques et de ce dossier de preuves ; il reste dans l’archive locale initiale.',
          '- L’empreinte du collecteur n’a pas été enregistrée au démarrage de la campagne. Le script conservé est identique à la copie de l’archive initiale ; le manifeste ultérieur protège cette copie sans constituer une attestation cryptographique de son exécution.', '',
          'Ce dossier conserve les preuves de la campagne. Son état de sauvegarde distante doit être vérifié par la référence Git distante ; la présence de ces fichiers ne constitue pas à elle seule une sauvegarde hors machine.', '']
lines += ['## Formulation utilisable avec son contexte', '',
          '> On an Intel Core i7-6700HQ running Windows 11 IoT Enterprise LTSC, Wardoff 0.2.0 averaged approximately 14 MiB resident memory and less than 0.01% total-system CPU in Block mode across three five-minute background runs. UpdateOrchestrator task protection was unavailable because the task was absent.', '',
          'Cette formulation décrit une mesure du processus sur cette machine ; elle ne promet ni la même consommation partout, ni une protection contre tous les arrêts Windows.', '']
(root/'RAPPORT.md').write_text('\n'.join(lines),encoding='utf-8')
print(json.dumps([s for s in summaries if s['scope']=='pooled'],indent=2))
