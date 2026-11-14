# Migration de Steam Press 0.x vers 1.0

## Compatibilité

Les fichiers créés par la dernière version 0.x restent lisibles en 1.0 :

| Format | Fichier | Migration |
|---|---|---|
| Réglages | `settings.ini` | `formatversion=1` ajouté ; un fichier sans ce champ est lu avec les valeurs par défaut. |
| Noms de dépôts | `depots.ini` | commentaire `; steam-press-format=1` ajouté ; les fichiers sans commentaire restent valides. |
| Identifiants | `credentials.bin` | préfixe `SPV1:` ajouté ; les anciens fichiers sans préfixe restent déchiffrables. |
| Checkpoints | `.steam-press-state.json` | champ `version` ajouté ; un fichier sans version est accepté. |
| File de jeux | `.sspq` | en-tête commentaire `; steam-press-format=1` ; les anciennes lignes restent valides. |
| File Workshop | `.sswq` | même en-tête commentaire ; les lignes `workshop_download_item` restent acceptées. |
| Historique GUI | `history.json` | nouveau conteneur `{formatVersion, entries}` ; l'ancien tableau brut reste lu. |
| Templates | fichier texte | tokens inchangés. |
| ACF | `appmanifest_*.acf` | format Steam, inchangé. |

## Changements de comportement

- `diff --archive` inclut désormais `deleted_files.txt` dans l'archive quand
  des fichiers ont été supprimés. Le rapport JSON contient déjà `removed`.
- La sortie `--json` est enveloppée dans un objet avec `schemaVersion: 1`.
  `search` renvoie `{schemaVersion, ok, results}` au lieu d'un tableau brut.
- Les archives Linux officielles sont compilées sur Ubuntu 22.04 et demandent
  glibc 2.35 ou plus récent.
- Les messages d'erreur du CLI suivent les codes documentés dans
  [docs/exit-codes.md](exit-codes.md).

## Procédure

1. Sauvegardez `~/.config/steam-press/` (ou `STEAM_PRESS_HOME`).
2. Installez la 1.0.
3. Lancez `steam-press doctor` puis `steam-press queue list`.
4. Ouvrez la GUI une fois : les files et l'historique sont migrés à la
   sauvegarde suivante.
5. Si vous utilisez des scripts, ajoutez `schemaVersion` à vos vérifications et
   adaptez `search --json` au nouveau conteneur.

Aucune commande de migration manuelle n'est nécessaire.
