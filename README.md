# Picture-swipe (Bildesveip)

En liten Mac-app for å rydde i bilder, skrevet i ren Rust. Du ser ett bilde om gangen og sveiper det til høyre for å beholde eller til venstre for å kaste. Ingenting slettes mens du sorterer: alt du kaster, samles i en egen visning, der du går gjennom det før noe faktisk forsvinner.

*A small macOS app for tidying photos, one swipe at a time, written in pure Rust. Discarded photos are only marked and collected for review, and nothing is removed until you confirm. The app's interface is in Norwegian.*

## Hva den gjør

- **Sorter med tastatur, mus eller styreflate.** Ett bilde om gangen, med angre.
- **Egen visning for «Til sletting».** Se gjennom alt du har kastet, og ombestem deg før noe skjer.
- **Bildemapper.** HEIC, JPEG, PNG, TIFF, WebP, AVIF og vanlige RAW-formater.
- **Bilder-biblioteket på Mac.** Sorter direkte i biblioteket ditt, også bilder som bare ligger i iCloud.
- **Laget for Mac.** Native menylinje, lys og mørk modus, og aksentfargen du har valgt i macOS.

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
| `⌘A` | Marker alt |
| `⌘1`, `⌘2`, `⌘3` | Sveip, Til sletting, Beholdt |
| `⌘O` | Åpne mappe |
| `⇧⌘O` | Åpne Bilder-biblioteket |

## Laget med

| Del | Teknologi |
|---|---|
| Språk | Rust |
| Grensesnitt | [egui](https://github.com/emilk/egui) og eframe, tegnet med wgpu |
| Bilder-biblioteket | Apples PhotoKit, via [objc2-photos](https://crates.io/crates/objc2-photos) |
| Menylinje | [muda](https://crates.io/crates/muda) |
| Bildekonvertering | macOS' eget `sips`-verktøy |

Koden er skrevet med hjelp av AI-verktøyene Claude Code og Codex.

## Bygg den selv

Appen er foreløpig bare for macOS 11 eller nyere, og det finnes ingen ferdig nedlasting. Du trenger [Rust](https://rustup.rs) og Xcodes kommandolinjeverktøy.

```bash
./scripts/bundle.sh                   # bygger target/release/Bildesveip.app
open target/release/Bildesveip.app
```

Appen signeres bare lokalt, så macOS kan spørre om tilgang til Bilder på nytt etter hver bygging.

Tester og kodekontroll:

```bash
cargo test
cargo clippy --all-targets -- -D warnings
```

Valgene dine lagres per mappe i `~/Library/Application Support/io.github.sti90.bildesveip/`.

## Begrensninger

- Bare øverste nivå i en mappe leses, ikke undermapper.
- Videoer og bildeserier fra Bilder-biblioteket er ikke med.
- Støtten for Bilder-biblioteket er ny, så prøv den gjerne på ett uviktig bilde først.

## Lisens

[MIT](LICENSE)
