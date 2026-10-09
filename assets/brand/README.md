# VSCLI mark

The original VSCLI mark is a forward terminal chevron made from two folded blue planes and a mint cursor. It is project artwork under the repository's MIT OR Apache-2.0 license, not the Visual Studio Code logo.

`vscli-mark.svg` is the editable source. The transparent 512 × 512 PNG is generated from that source and embedded by the native editor:

```sh
rsvg-convert -w 512 -h 512 assets/brand/vscli-mark.svg -o assets/brand/vscli-mark.png
```

No external converter or asset file is needed to run VSCLI. The runtime PNG decoder is native, size-limited and restricted to PNG. The raster asset must remain at most 64 KiB and 512 × 512 pixels; tests enforce both bounds.
