# Codes de sortie du CLI

| Code | Nom | Signification |
|---:|---|---|
| 0 | `SUCCESS` | Commande terminée, job réussi ou ignoré volontairement. |
| 1 | `FAILURE` | Erreur d'exécution : Steam, téléchargement, disque, archive, manifeste, template ou configuration. |
| 2 | `USAGE` | Arguments invalides ou commande inconnue (généré par Clap). |

Les erreurs de `pack`, `workshop run` et des commandes de file utilisent `1`
dès qu'au moins un job échoue ou est annulé. `--json` conserve le code de sortie
et ajoute `schemaVersion`, `ok` et `error` si applicable.

## Exemples

```bash
steam-press doctor --json; echo $?          # 0
steam-press depots lookup 0; echo $?        # 1, dépôt inconnu
steam-press pack --queue absent.sspq; echo $? # 0 si la file absente est vide
steam-press --commande-inconnue; echo $?     # 2
```

Les codes sont stables depuis la 1.0 ; un nouveau code ne sera introduit que
dans une version majeure.
