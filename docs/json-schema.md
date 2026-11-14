# Schéma de sortie `--json`

Toutes les commandes qui acceptent `--json` produisent un objet JSON avec :

```json
{
  "schemaVersion": 1,
  "ok": true
}
```

`ok` vaut `false` et `error` contient un message quand la commande échoue.
Le code de sortie reste la source de vérité (voir `exit-codes.md`).

## Commandes

| Commande | Champs principaux |
|---|---|
| `doctor` | `configDir`, `workDir`, `outputDir`, `outputWritable`, `archiver`, `steamcmd`, `depotdownloader`, `sevenzip`, `rar`, `knownDepots`, `diskFreeBytes`, `credentials`, `network` |
| `search` | `results[]` avec `appid`, `name` |
| `info` | `appid`, `name`, `installDir`, `buildId`, `buildTime`, `depots[]` (`depotId`, `name`, `manifestId`, `dlcAppid`, `size`, `sharedOwner`), `store` |
| `login` | `anonymous` ou `username` |
| `queue list` | `file`, `jobs[]` (`platform`, `appid`, `branch`, `protected`), `warnings[]` |
| `queue add/remove/move/clear/import/export` | `file`, `count`, et `added` pour l'import |
| `workshop list` | `file`, `jobs[]` (`appid`, `workshopId`), `warnings[]` |
| `workshop add/import/export` | `file`, `count`, `added` |
| `depots sync` | `count` |
| `depots lookup` | `depotId`, `name` |
| `depots list` | `count`, `depots[]` |
| `pack` | `jobs[]` (`job`, `status`, `error`, `output`, `infoFiles`, `buildId`, `durationMs`, `depots[]`) |
| `template render` | `rendered` |
| `import` | `job`, `status`, `output`, `manifest`, `copiedBytes`, `durationMs` |
| `diff` | `stem`, `report` (`added`, `modified`, `removed`, `unchanged`), `reportPath`, `output` |

Les champs sont en camelCase. Les tailles sont en octets et les durées en
millisecondes. Les chemins sont des chaînes dépendantes de la plateforme.

## Compatibilité

Le champ `schemaVersion` doit être vérifié par les scripts. Un changement de
champ incompatible incrémente cette version. Les ajouts de champs sont
compatibles et ne changent pas la version.
