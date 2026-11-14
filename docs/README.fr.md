# Vinyl AirPlay

[English](../README.md) · [Documentation](README.md)

Diffuse une platine ou une autre entrée audio Linux vers des HomePod, avec un
contrôle depuis le navigateur. Le téléphone sert de télécommande et ne transporte
pas le son.

**Version alpha.** Testée avec HomePod de première génération et HomePod mini.
Voir la [compatibilité et les limites connues](COMPATIBILITY.md).

## Fonctionnalités

- Connexion automatique à la lecture et déconnexion après silence.
- Détection des enceintes et réglage de leur volume.
- Gain fixe, niveaux stéréo et spectre audio.
- Reconnaissance facultative des morceaux et affichage de la pochette.
- Fonctionnement local, sans compte ni abonnement.

## Démarrage rapide

Prérequis : **Linux**, **Rust/Cargo**, **alsa-utils**, **Python 3.10+** et une entrée
ligne stéréo. Les enceintes doivent être sur le même réseau.

```sh
git clone https://github.com/bastiencantet/vinyl-airplay.git
cd vinyl-airplay
cargo build --release --locked
arecord -L
```

Remplace `YOUR_ALSA_INPUT` par le périphérique listé par `arecord -L` :

```sh
python3 scripts/serve-web.py --alsa-device YOUR_ALSA_INPUT
```

Ouvre **http://localhost:8787/** sur l'ordinateur Linux. Sélectionne une enceinte,
applique puis démarre. Sur une installation neuve, la diffusion et la
reconnaissance sont désactivées.

## Guides

- [Installation et branchement](INSTALLATION.md)
- [Utilisation](USAGE.md)
- [Configuration et accès depuis un téléphone](CONFIGURATION.md)
- [Dépannage](TROUBLESHOOTING.md)

Pour participer : [contributions](../CONTRIBUTING.md),
[discussions](https://github.com/bastiencantet/vinyl-airplay/discussions) et
[feuille de route](../ROADMAP.md).

Licence [GPL-3.0-or-later](../LICENSE). Projet indépendant d'Apple.
