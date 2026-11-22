# Handoff — Steam Press 1.0.3

État au **2026-11-21**. Le lot fonctionnel est le commit `9927171` ; le présent document
est ajouté dans le commit de passation qui suit, sur la branche
`feat/1.0.3-json-contracts`.

> **Mise à jour du 2026-11-22** — la connexion Steam Guard est maintenant implémentée
> sur la branche `feat/1.0.3-steam-guard` : `--steam-guard-code` pour `pack` et
> `workshop run`, prompt interactif unique, champ et bouton **Retry** dans la GUI,
> transmission du code à DepotDownloader par l'entrée standard. Les vérifications
> `fmt`, `clippy` et `cargo test --workspace` passent. La procédure PR / CodeRabbit /
> merge / release ci-dessous reste la référence.

## En une phrase

Le code, les tests et la documentation de la 1.0.3 sont prêts et poussés. Il reste à
**créer la PR**, laisser CodeRabbit la relire, corriger les remarques valides, merger, puis
vérifier la release automatique `v1.0.3`.

## État Git exact

- Dépôt : `Matheotaku4/Steam-Press`
- Branche de travail : `feat/1.0.3-json-contracts`
- Base : `main` à `647426b` (`Rename coderabbit.yaml to .coderabbit.yaml`)
- Commit fonctionnel du lot : `9927171` (`test: snapshot the network-backed JSON contracts`)
- Commit de passation : celui qui contient ce document (`docs: add 1.0.3 handoff`)
- Branche distante : poussée
- PR : **pas encore créée** (voir « Blocage principal »)
- Release `v1.0.3` : **pas publiée**, pas de tag `v1.0.3`
- Dernière release publiée : `v1.0.2`
- Seul élément non suivi localement : `artifacts/` (captures sources, à ne pas committer)

Comparer la branche :

<https://github.com/Matheotaku4/Steam-Press/compare/main...feat/1.0.3-json-contracts?expand=1>

## Ce qui a été livré dans `9927171`

| Fichier | Changement |
|---|---|
| `crates/steam-press-cli/src/commands.rs` | Constructeurs JSON purs pour `doctor`, `search`, `info`, `pack`, `import`, avec snapshots unitaires exacts. |
| `crates/steam-press-cli/tests/json_snapshots.rs` | Inchangé : couvre déjà `queue list`, `workshop list`, `depots list`, `login`, `template render`, `diff` et les erreurs CLI. |
| `docs/json-schema.md` | Décrit les 11 commandes et les cinq nouveaux contrats hors ligne. |
| `docs/usage.md` | Nouvelle section « Aperçu de la GUI » avec six captures. |
| `docs/screenshots/` | `library.png`, `workshop.png`, `history.png`, `depots.png`, `settings.png`, `light-theme.png`. |
| `.github/workflows/release.yml` | Les trois archives embarquent maintenant `docs/screenshots/`. |
| `Cargo.toml`, `Cargo.lock` | Version `1.0.3` pour core, CLI et GUI. |
| `CHANGELOG.md`, `README.md` | Entrée 1.0.3, exemple de tag mis à jour. |

Les builders sont aux lignes ~108-193 de `commands.rs` ; les snapshots commencent vers la
ligne ~1115. Les captures proviennent de la GUI **1.0.2** sous X11/Xvfb ; la GUI 1.0.3 n’a
pas été relancée dans ce lot (aucun code GUI modifié).

## Vérifications réellement exécutées

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo build --locked -p steam-press-cli
./target/debug/steam-press --version   # steam-press 1.0.3
```

Résultats :

- core : **88 tests**
- CLI : **9 tests** + **4 snapshots d’intégration**
- GUI : **19 tests**, 2 ignorés (réseau / release publiée)
- fmt : OK ; clippy : OK
- `git diff --check` : OK

Exemples exécutés hors ligne ou en réseau réel lecture seule, avec des répertoires
temporaires et `--json` validé par `python3 -c 'json.load(...)'` :

- `doctor` (réseau Steam disponible)
- `search "half-life"`
- `info 2379780`
- `queue add 2379780 --file ...` puis `queue list --file ...`
- `depots list`
- `diff <old> <new>`
- `template render` est couvert par le test d’intégration existant

Non exécuté dans ce lot : installation réelle de l’updater, RAR, `workshop run`, job réel
complet depuis la GUI, release locale des trois OS.

## Blocage principal : la PR n’existe pas

Le push de la branche a réussi, mais aucun outil de ce chat ne peut créer une PR :

- `gh auth status` répond **not logged into any GitHub hosts** ;
- aucun outil d’intégration GitHub n’est disponible (seul Linear est connecté) ;
- le navigateur cloud arrive sur GitHub **déconnecté** ;
- `cloud_git` ne sait que fetch / pull / push.

Il ne faut pas essayer de récupérer un token GitHub, ni pousser directement sur `main`.
Le prochain agent doit soit avoir une session GitHub authentifiée, soit demander à
l’utilisateur de cliquer sur le bouton de création de PR.

Lien direct :

<https://github.com/Matheotaku4/Steam-Press/pull/new/feat/1.0.3-json-contracts>

## Procédure de reprise

### 1. Créer la PR

Titre suggéré :

```text
chore(release): 1.0.3 — JSON contracts and GUI docs
```

Corps suggéré (à adapter si CodeRabbit ou le contexte l’exige) :

```markdown
## Résumé

- ajoute des snapshots hors ligne pour `doctor`, `search`, `info`, `pack` et `import` ;
- documente les 11 contrats `--json` et leur stabilité ;
- intègre les captures GUI au guide et aux archives de release ;
- prépare la version 1.0.3.

## Vérifications

- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets --locked -- -D warnings`
- `cargo test --workspace --locked`
- `steam-press --version` → 1.0.3
- exemples `doctor`, `search`, `info`, `queue`, `depots`, `diff` exécutés

## Tickets

Fixes IRD-83
Part of IRD-88
Refs IRD-77
```

`Fixes IRD-83` est justifié : le schéma des 11 commandes est documenté et chaque commande
a un test de non-régression. Ne pas ajouter `Fixes IRD-88` : RAR et `workshop run` ne sont
toujours pas exécutés.

### 2. CodeRabbit

La configuration `.coderabbit.yaml` est déjà sur `main` (donc dans la base de la PR) :
profil `assertive`, review détaillée, consignes en français.

- Attendre la review automatique après création de la PR.
- Si elle ne démarre pas, commenter `@coderabbitai review`.
- Ne corriger que les remarques techniquement valides.
- Ne pas affaiblir un test, désactiver un lint ou fermer un ticket pour faire passer la CI.
- Relancer les checks après chaque correction.

### 3. Merger

Attendre :

- CI GitHub verte (`.github/workflows/ci.yml`) ;
- review CodeRabbit sans remarque valide non traitée ;
- aucun conflit avec `main`.

Merger dans `main` (squash ou merge commit, selon l’habitude du dépôt). Le workflow
`.github/workflows/release.yml` se déclenche sur `push` vers `main`. Il lit la version dans
`crates/steam-press-core/Cargo.toml` ; comme `v1.0.3` n’existe pas, il construit et publie.

Un tag manuel n’est pas nécessaire. Si le workflow `main` échoue ou si la publication est
relancée depuis un tag :

```bash
git switch main
git pull --ff-only
git tag -a v1.0.3 -m "Steam Press 1.0.3"
git push origin v1.0.3
```

Ne pas créer le tag avant le merge : cela publierait depuis la branche, pas depuis `main`.

Si `gh release create` échoue après avoir créé le tag, une relance voit la release
existante et `prepare` passe `publish=false`. Récupérer avec :

```bash
gh release delete v1.0.3 --cleanup-tag --yes
gh workflow run release.yml --ref main
```

ou téléverser les archives manquantes sur la release existante avec
`gh release upload v1.0.3 dist/*.tar.gz dist/*.sha256 dist/SHA256SUMS --clobber`.

### 4. Vérifier la release

```bash
gh run list --workflow Release --limit 5
gh release view v1.0.3 --json tagName,isDraft,assets
```

Attendu :

- release `v1.0.3` non draft ;
- **12 archives** : 3 formats × 4 plateformes (`linux-x86_64`, `macos-x86_64`,
  `macos-aarch64`, `windows-x86_64`) ;
- **12 fichiers `.sha256`** ;
- **`SHA256SUMS`** ;
- documentation et `docs/screenshots/` présents dans les trois archives Linux.

Vérification téléchargeable :

```bash
mkdir -p /tmp/sp-1.0.3 && cd /tmp/sp-1.0.3
gh release download v1.0.3 --repo Matheotaku4/Steam-Press
sha256sum -c ./*.sha256
sha256sum -c SHA256SUMS
tar -xzf steam-press-cli-1.0.3-linux-x86_64.tar.gz
./steam-press-cli-1.0.3-linux-x86_64/steam-press --version   # 1.0.3
tar -tzf steam-press-cli-1.0.3-linux-x86_64.tar.gz | grep 'docs/screenshots/'
```

### 5. Linear

- **IRD-83** : actuellement *In Review*. Passer à *Done* après merge et CI verte.
- **IRD-88** : actuellement *In Progress*. Laisser ouvert : captures intégrées, exemples
  partiels exécutés, mais RAR, `workshop run` et l’exhaustivité des exemples manquent.
- **IRD-77** : parent ; ne pas fermer tant que les tickets bloqués ci-dessous restent ouverts.

Tickets bloqués / hors périmètre à ne pas fermer arbitrairement :

| Ticket | Raison |
|---|---|
| IRD-78 | SteamCMD réel et cas de compte (Steam Guard/mauvais mot de passe) impossibles sans runtime 32 bits et compte. |
| IRD-79 | Pas de Wayland/Windows/macOS ni job réel complet depuis la GUI ; seul X11/Xvfb 1.0.2 est validé. |
| IRD-80 | L’installation réelle de l’updater est volontairement exclue. |
| IRD-84 | Aucune machine Windows/macOS disponible. |
| IRD-88 | RAR et `workshop run` non exécutés ; tous les exemples ne sont pas prouvés. |
| IRD-92 | Aucune release candidate publiée ; bloquants encore ouverts. |
| IRD-77 | Ticket parent, dépend des points ci-dessus. |
| IRD-97/94/76/51 | Quark launcher, hors périmètre Steam Press. |

## Contraintes à conserver

- Documentation en français ; code, UI et messages CLI en anglais.
- Ne pas pousser directement sur `main` pour ce lot : passer par la PR.
- Ne pas fermer un ticket dont les critères ne sont pas réellement satisfaits.
- Ne pas installer réellement l’updater ni altérer l’environnement pour le tester.
- Ne pas committer `artifacts/` : les captures utiles sont déjà dans `docs/screenshots/`.
- Ne pas modifier `.coderabbit.yaml` sans raison ; il est sur `main`.
- Ne jamais écrire de token ou de secret dans le dépôt, un fichier ou un commentaire.

## Définition de terminé

- PR mergée dans `main`, CI verte, remarques CodeRabbit valides traitées.
- Release `v1.0.3` publiée avec 12 archives, 12 checksums et `SHA256SUMS` vérifiés.
- CLI Linux 1.0.3 exécuté depuis l’archive et checksum validé.
- IRD-83 passé à *Done* ; IRD-88 laissé ouvert avec un commentaire de suivi honnête.
