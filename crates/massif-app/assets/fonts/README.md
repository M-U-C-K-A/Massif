# Typefaces

The typefaces Settings ▸ Appearance offers besides the system's own, built
into Massif. Each is a variable font with a weight axis, so the interface's
medium and semi-bold weights are real ones. All four are under the SIL Open
Font License 1.1, whose text sits beside each font.

| File | Typeface | Source |
| --- | --- | --- |
| `Inter.ttf` | Inter (opsz, wght) | <https://github.com/rsms/inter>, via <https://github.com/google/fonts/tree/main/ofl/inter> |
| `Geist.ttf` | Geist (wght) | <https://github.com/vercel/geist-font>, via <https://github.com/google/fonts/tree/main/ofl/geist> |
| `IBMPlexSans.ttf` | IBM Plex Sans (wdth, wght) | <https://github.com/IBM/plex>, via <https://github.com/google/fonts/tree/main/ofl/ibmplexsans> |
| `Manrope.ttf` | Manrope (wght) | <https://github.com/sharanda/manrope>, via <https://github.com/google/fonts/tree/main/ofl/manrope> |

The files are the ones Google Fonts publishes. Geist and IBM Plex Sans are
unchanged. Inter and Manrope have lost their private-use code points
(U+E000–U+F8FF) and the glyphs only those reached, nothing else — their
features, alternates, weights and names stay: Lucide's icons, which Massif
writes inline in its labels, sit at the same code points and would
otherwise be drawn with those glyphs. Made with fontTools' subsetter,
keeping every other code point, every layout feature and every name.
Neither font has a Reserved Font Name.
