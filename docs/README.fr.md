# Vinyl AirPlay

[English](../README.md) · [Documentation](README.md)

Diffuse une platine, une interface USB ou un microphone vers des HomePod, avec un
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

Prérequis : **Rust/Cargo**, **Python 3.10+** et une entrée audio. Linux nécessite
aussi les en-têtes de développement ALSA et `pkg-config`. Les enceintes doivent
être sur le même réseau. Le backend natif est choisi à la compilation pour Linux,
macOS ou Windows ; les essais audio sur matériel restent validés sous Linux.

```sh
git clone https://github.com/bastiencantet/vinyl-airplay.git
cd vinyl-airplay
cargo build --release --locked
./target/release/vinyl-airplay audio-inputs
```

Lance avec l’entrée par défaut, ou ajoute `--input-device IDENTIFIANT` :

```sh
python3 scripts/serve-web.py
```

Ouvre **http://localhost:8787/** sur l’ordinateur qui diffuse. Sélectionne une enceinte,
applique puis démarre. Sur une installation neuve, la diffusion et la
reconnaissance sont désactivées.

Sous Windows, utilise `python` et les exécutables `.exe`. Le chemin Linux
historique reste disponible avec `--alsa-device YOUR_ALSA_INPUT`.

## Guides

- [Installation et branchement](INSTALLATION.md)
- [Utilisation](USAGE.md)
- [Configuration et accès depuis un téléphone](CONFIGURATION.md)
- [Dépannage](TROUBLESHOOTING.md)

Pour participer : [contributions](../CONTRIBUTING.md),
[discussions](https://github.com/bastiencantet/vinyl-airplay/discussions) et
[feuille de route](../ROADMAP.md).

Licence [GPL-3.0-or-later](../LICENSE). Projet indépendant d'Apple.
