Test fixtures for `src/lerc.rs` and `src/cog.rs`, made with Esri's LERC library through
Python's imagecodecs and tifffile (`tools/testdata/make_lerc.py` in the commit history):
a 64 × 48 float32 surface, `-12 + 0.3x - 0.15y + 2 sin(0.21x) cos(0.17y)`, encoded with
a maximum error of 0.01 as Lerc2 versions 2, 3 and 4, with and without a mask (a disc of
radius 6 round (40, 20) and the first three columns invalid), and losslessly.
