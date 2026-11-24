**Exports onto a USB drive from a Mac open as made**

On a drive formatted for Windows (exFAT or FAT, as most USB drives and SD cards come), macOS writes a hidden "._" file beside every file it writes. Minecraft read the one beside the height data pack's overworld as a part of the pack, failed on it, and opened the world without the pack, cut at its own height. The export now removes those files from the world it writes, and from the data packs it copies in: a world exported onto such a drive opens with its height.
