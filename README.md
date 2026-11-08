# Picture-swipe (Bildesveip)

En liten Mac-app for å rydde i bilder. Du ser ett bilde om gangen og sveiper det til høyre for å beholde eller til venstre for å kaste. Ingenting slettes mens du sorterer: alt du kaster, samles i en egen visning, der du går gjennom det før noe faktisk forsvinner.

*A small macOS app for tidying photos, one swipe at a time. Discarded photos are only marked and collected for review, and nothing is removed until you confirm. The app's interface is in Norwegian.*

## Hva den gjør

- **Sorter med tastatur, mus eller styreflate.** Ett bilde om gangen, med angre.
- **Egen visning for «Til sletting».** Se gjennom alt du har kastet, og ombestem deg før noe skjer.
- **Bildemapper.** HEIC, JPEG, PNG, TIFF, WebP, AVIF og vanlige RAW-formater.
- **Bilder-biblioteket på Mac.** Sorter direkte i biblioteket ditt, også bilder som bare ligger i iCloud.
- **Laget for Mac.** Følger Apples retningslinjer for design, med systemfarger, lys og mørk modus og full menylinje.

## Slik beskytter den bildene dine

Sletting er det eneste som ikke skal kunne gå galt, så det er bygget i flere lag:

- Å kaste et bilde er bare et merke. Filen blir liggende urørt.
- Ingenting flyttes før du bekrefter fra «Til sletting».
- Bilder fra mapper går til papirkurven. Bilder fra biblioteket går til «Nylig slettet», etter en ekstra bekreftelse fra macOS. Appen sletter aldri permanent.
- Merket er knyttet til akkurat den filen du så. Bytter noen ut filen, må den vurderes på nytt.
- Appen sender ingenting ut fra maskinen. Bilder fra iCloud hentes av macOS selv.

**Merk:** Sletter du fra Bilder-biblioteket med iCloud-bilder på, forsvinner bildene fra alle enhetene dine. Bilder i et delt bibliotek fjernes også for de andre.

## Taster

| Tast | Handling |
|---|---|
| `→` | Behold |
| `←` | Kast |
| `↓` | Hopp over |
| `⌘Z` | Angre |
| `Mellomrom` | Stor visning |
| `B` | Legg tilbake, i «Til sletting» og «Beholdt» |
| `⌘1`, `⌘2`, `⌘3` | Sveip, Til sletting, Beholdt |
| `⌘O` | Åpne mappe |
| `⇧⌘O` | Åpne Bilder-biblioteket |

## Laget med

| Del | Teknologi |
|---|---|
| App-rammeverk | [Tauri 2](https://tauri.app) |
| Backend | Rust |
| Grensesnitt | TypeScript og CSS, uten UI-rammeverk |
| Bilder-biblioteket | Apples PhotoKit, via en liten Objective-C-bro |
| Miniatyrer | macOS' eget `sips`-verktøy |

Den ferdige appen er rundt 5 MB. Koden er skrevet med hjelp av AI-verktøyene Claude Code og Codex.

## Bygg den selv

Appen er foreløpig bare for macOS 11 eller nyere, og det finnes ingen ferdig nedlasting. Du trenger [Rust](https://rustup.rs), [Node.js](https://nodejs.org) og Xcodes kommandolinjeverktøy.

```bash
npm install
npm run tauri dev                    # kjør i utviklingsmodus
npm run tauri build -- --bundles app # bygg Bildesveip.app
```

Appen signeres bare lokalt, så macOS kan spørre om tilgang til Bilder på nytt etter hver bygging.

Tester og kodekontroll kjøres i `src-tauri`:

```bash
cargo test
cargo clippy --all-targets -- -D warnings
```

`CONTRACT.md` beskriver grensesnittet mellom Rust og TypeScript, sikkerhetsreglene og de kjente begrensningene. Valgene dine lagres per mappe i `~/Library/Application Support/io.github.sti90.bildesveip/`. Ikonet tegnes av `design/lag-ikon.swift`.

## Begrensninger

- Bare øverste nivå i en mappe leses, ikke undermapper.
- Videoer og bildeserier fra Bilder-biblioteket er ikke med.
- Støtten for Bilder-biblioteket er ny, så prøv den gjerne på ett uviktig bilde først.

## Lisens

[MIT](LICENSE)
