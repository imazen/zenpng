# benches/pareto.rs on arm-xl (Neoverse-N1, cores 2-9, nice 19), encode ST only, RGB8+RGBA8 at 256 and 1024 px (25 images per size).
# zenpng tree 2026-10-06 ~11:26 UTC with the release roundtrip check already off; zenflate snapshot: zenflate 60f3d5e7e7bbd45bc2df2e5b98d7f70d48f06c5d dirty:  M src/decompress/mod.rs  M src/decompress/streaming.rs  M tests/fuzz_regression.rs 
# run1: current ladder. run2: ZENPNG_LADDER_E<n> candidates:
#   E1 paeth|g1  E2 minsum|p1  E3 paeth|p4  E4 minsum|p4  E5 minsum|p6  E6 minsum|p8  E7 minsum|p10  E8 minsum|p12
#   E9 minsum|g17  E10 minsum|p12|g17|1  E11 fast|p4|p12|1  E12 heuristic|p4|p12|1  E13 heuristic|p4|p12,g17|2

## run1 (current ladder)
## enc

| arm | n | ms = a + b·MP (a ms, b ms/MP) | bytes = a + b·MP (enc) |
|---|---|---|---|
| lodepng | 50 | -0.148 + 123.287·MP | 34609 + 865310·MP |
| png_balanced | 50 | 0.475 + 75.103·MP | 35617 + 878531·MP |
| png_fast | 50 | 0.000 + 9.512·MP | 28635 + 1090789·MP |
| png_high | 50 | -11.175 + 406.216·MP | 34996 + 847562·MP |
| zenpng_e13_st | 50 | 10.620 + 789.807·MP | 34093 + 851282·MP |
| zenpng_e1_st | 50 | 0.676 + 23.336·MP | 32478 + 959617·MP |
| zenpng_e2_st | 50 | 2.024 + 104.287·MP | 31806 + 947025·MP |
| zenpng_e3_st | 50 | 3.609 + 174.077·MP | 31683 + 946175·MP |
| zenpng_e5_st | 50 | 3.884 + 190.244·MP | 32194 + 937466·MP |
| zenpng_e7_st | 50 | 3.926 + 189.880·MP | 32280 + 936874·MP |
| zenpng_e9_st | 50 | 8.095 + 355.866·MP | 32642 + 895469·MP |
| zune | 50 | -0.126 + 4.682·MP | -10195 + 3242187·MP |

### enc by size_class: median time / png_high time, median size / png_high size

**medium** (n=25)

| arm | time | size | dominated by |
|---|---|---|---|
| zune | 0.016 | 3.7228 |  |
| png_fast | 0.031 | 1.2117 |  |
| zenpng_e1_st | 0.085 | 1.1242 |  |
| png_balanced | 0.192 | 1.0309 |  |
| lodepng | 0.334 | 1.0142 |  |
| zenpng_e2_st | 0.415 | 1.0917 | lodepng, png_balanced |
| zenpng_e3_st | 0.670 | 1.0917 | lodepng, png_balanced |
| zenpng_e5_st | 0.748 | 1.0833 | lodepng, png_balanced |
| zenpng_e7_st | 0.750 | 1.0833 | lodepng, png_balanced |
| png_high | 1.000 | 1.0000 |  |
| zenpng_e9_st | 1.456 | 1.0275 | lodepng, png_high |
| zenpng_e13_st | 2.647 | 1.0052 | png_high |

**small** (n=25)

| arm | time | size | dominated by |
|---|---|---|---|
| zune | 0.016 | 3.0251 |  |
| png_fast | 0.045 | 1.1826 |  |
| zenpng_e1_st | 0.094 | 1.1114 |  |
| png_balanced | 0.269 | 1.0463 |  |
| lodepng | 0.383 | 1.0234 |  |
| zenpng_e2_st | 0.421 | 1.0876 | lodepng, png_balanced |
| zenpng_e3_st | 0.765 | 1.0866 | lodepng, png_balanced |
| zenpng_e7_st | 0.847 | 1.0837 | lodepng, png_balanced |
| zenpng_e5_st | 0.853 | 1.0837 | lodepng, png_balanced |
| png_high | 1.000 | 1.0000 |  |
| zenpng_e9_st | 1.452 | 1.0302 | lodepng, png_high |
| zenpng_e13_st | 2.919 | 0.9980 |  |

### enc by content: median time / png_high time, median size / png_high size

**document** (n=12)

| arm | time | size | dominated by |
|---|---|---|---|
| zune | 0.018 | 5.4439 |  |
| png_fast | 0.039 | 1.2275 |  |
| zenpng_e1_st | 0.093 | 1.1441 |  |
| png_balanced | 0.251 | 1.0714 |  |
| lodepng | 0.337 | 1.0662 |  |
| zenpng_e2_st | 0.437 | 1.1229 | lodepng, png_balanced |
| zenpng_e3_st | 0.818 | 1.1155 | lodepng, png_balanced |
| zenpng_e7_st | 0.887 | 1.1039 | lodepng, png_balanced |
| zenpng_e5_st | 0.888 | 1.1063 | lodepng, png_balanced |
| png_high | 1.000 | 1.0000 |  |
| zenpng_e9_st | 1.568 | 1.0202 | png_high |
| zenpng_e13_st | 2.921 | 0.9950 |  |

**lineart** (n=8)

| arm | time | size | dominated by |
|---|---|---|---|
| zune | 0.017 | 7.5150 |  |
| png_fast | 0.038 | 1.6021 |  |
| zenpng_e1_st | 0.087 | 1.2015 |  |
| png_balanced | 0.192 | 1.0874 |  |
| lodepng | 0.280 | 1.0511 |  |
| zenpng_e2_st | 0.368 | 1.1929 | lodepng, png_balanced |
| zenpng_e3_st | 0.581 | 1.1887 | lodepng, png_balanced |
| zenpng_e5_st | 0.595 | 1.1719 | lodepng, png_balanced |
| zenpng_e7_st | 0.596 | 1.1639 | lodepng, png_balanced |
| zenpng_e9_st | 0.803 | 1.0453 |  |
| png_high | 1.000 | 1.0000 |  |
| zenpng_e13_st | 1.449 | 1.0149 | png_high |

**mixed** (n=6)

| arm | time | size | dominated by |
|---|---|---|---|
| zune | 0.017 | 1.5596 |  |
| png_fast | 0.056 | 1.1160 |  |
| zenpng_e1_st | 0.175 | 1.0428 |  |
| png_balanced | 0.470 | 1.0120 |  |
| lodepng | 0.594 | 1.0056 |  |
| zenpng_e2_st | 0.848 | 1.0409 | lodepng, png_balanced |
| png_high | 1.000 | 1.0000 |  |
| zenpng_e3_st | 1.425 | 1.0390 | lodepng, png_balanced, png_high |
| zenpng_e5_st | 1.556 | 1.0346 | lodepng, png_balanced, png_high |
| zenpng_e7_st | 1.559 | 1.0346 | lodepng, png_balanced, png_high |
| zenpng_e9_st | 2.962 | 1.0216 | lodepng, png_balanced, png_high |
| zenpng_e13_st | 5.767 | 1.0059 | lodepng, png_high |

**photo** (n=18)

| arm | time | size | dominated by |
|---|---|---|---|
| zune | 0.008 | 2.4466 |  |
| png_fast | 0.027 | 1.1262 |  |
| zenpng_e1_st | 0.075 | 1.0880 |  |
| png_balanced | 0.221 | 1.0160 |  |
| zenpng_e2_st | 0.323 | 1.0761 | png_balanced |
| lodepng | 0.377 | 0.9987 |  |
| zenpng_e3_st | 0.555 | 1.0744 | lodepng, png_balanced |
| zenpng_e7_st | 0.607 | 1.0718 | lodepng, png_balanced |
| zenpng_e5_st | 0.607 | 1.0718 | lodepng, png_balanced |
| png_high | 1.000 | 1.0000 |  |
| zenpng_e9_st | 1.133 | 1.0312 | lodepng, png_balanced, png_high |
| zenpng_e13_st | 2.392 | 1.0013 | lodepng, png_high |

**screen** (n=6)

| arm | time | size | dominated by |
|---|---|---|---|
| zune | 0.023 | 7.2482 |  |
| png_fast | 0.045 | 1.5943 |  |
| zenpng_e1_st | 0.088 | 1.2671 |  |
| png_balanced | 0.222 | 1.1128 |  |
| lodepng | 0.376 | 1.1017 |  |
| zenpng_e2_st | 0.456 | 1.2246 | lodepng, png_balanced |
| zenpng_e3_st | 0.889 | 1.2246 | lodepng, png_balanced |
| zenpng_e7_st | 0.932 | 1.2059 | lodepng, png_balanced |
| zenpng_e5_st | 0.937 | 1.2066 | lodepng, png_balanced |
| png_high | 1.000 | 1.0000 |  |
| zenpng_e9_st | 1.574 | 1.0861 | png_high |
| zenpng_e13_st | 2.925 | 1.0205 | png_high |


## run2 (candidates)
## enc

| arm | n | ms = a + b·MP (a ms, b ms/MP) | bytes = a + b·MP (enc) |
|---|---|---|---|
| lodepng | 50 | -0.128 + 123.039·MP | 34625 + 865306·MP |
| png_balanced | 50 | 0.377 + 75.433·MP | 35630 + 878532·MP |
| png_fast | 50 | -0.005 + 9.549·MP | 28645 + 1090809·MP |
| png_high | 50 | -11.042 + 404.682·MP | 35013 + 847555·MP |
| zenpng_e10_st | 50 | 0.630 + 228.205·MP | 33405 + 863839·MP |
| zenpng_e11_st | 50 | 0.738 + 266.416·MP | 35756 + 865230·MP |
| zenpng_e12_st | 50 | 3.238 + 418.972·MP | 35762 + 864491·MP |
| zenpng_e13_st | 50 | 3.760 + 796.401·MP | 34304 + 849939·MP |
| zenpng_e1_st | 50 | 0.673 + 23.217·MP | 32483 + 959643·MP |
| zenpng_e2_st | 50 | 0.194 + 21.016·MP | 30042 + 1029690·MP |
| zenpng_e3_st | 50 | 0.284 + 27.443·MP | 34803 + 931829·MP |
| zenpng_e4_st | 50 | 0.261 + 39.113·MP | 33968 + 924985·MP |
| zenpng_e5_st | 50 | 0.535 + 46.073·MP | 33782 + 910068·MP |
| zenpng_e6_st | 50 | 0.490 + 51.351·MP | 34173 + 896303·MP |
| zenpng_e7_st | 50 | 0.415 + 63.148·MP | 34530 + 884165·MP |
| zenpng_e8_st | 50 | -0.016 + 77.781·MP | 34874 + 875732·MP |
| zenpng_e9_st | 50 | -0.536 + 112.897·MP | 33787 + 867019·MP |
| zune | 50 | -0.094 + 4.509·MP | -10233 + 3242405·MP |

### enc by size_class: median time / png_high time, median size / png_high size

**medium** (n=25)

| arm | time | size | dominated by |
|---|---|---|---|
| zune | 0.014 | 3.7228 |  |
| png_fast | 0.032 | 1.2117 |  |
| zenpng_e2_st | 0.062 | 1.1379 |  |
| zenpng_e1_st | 0.085 | 1.1242 |  |
| zenpng_e3_st | 0.097 | 1.0964 |  |
| zenpng_e4_st | 0.146 | 1.0923 |  |
| zenpng_e5_st | 0.184 | 1.0750 |  |
| png_balanced | 0.193 | 1.0309 |  |
| zenpng_e6_st | 0.205 | 1.0542 | png_balanced |
| zenpng_e7_st | 0.235 | 1.0403 | png_balanced |
| zenpng_e8_st | 0.254 | 1.0277 |  |
| lodepng | 0.336 | 1.0142 |  |
| zenpng_e9_st | 0.344 | 1.0193 | lodepng |
| zenpng_e10_st | 0.629 | 1.0193 | lodepng |
| zenpng_e11_st | 0.979 | 1.0201 | lodepng |
| png_high | 1.000 | 1.0000 |  |
| zenpng_e12_st | 1.646 | 1.0188 | lodepng, png_high |
| zenpng_e13_st | 2.675 | 1.0051 | png_high |

**small** (n=25)

| arm | time | size | dominated by |
|---|---|---|---|
| zune | 0.016 | 3.0251 |  |
| png_fast | 0.044 | 1.1826 |  |
| zenpng_e2_st | 0.091 | 1.1211 |  |
| zenpng_e1_st | 0.094 | 1.1114 |  |
| zenpng_e3_st | 0.120 | 1.0976 |  |
| zenpng_e4_st | 0.172 | 1.0769 |  |
| zenpng_e5_st | 0.201 | 1.0567 |  |
| zenpng_e6_st | 0.216 | 1.0557 |  |
| zenpng_e7_st | 0.250 | 1.0479 |  |
| png_balanced | 0.258 | 1.0463 |  |
| zenpng_e8_st | 0.280 | 1.0454 |  |
| zenpng_e9_st | 0.365 | 1.0165 |  |
| lodepng | 0.377 | 1.0234 |  |
| zenpng_e10_st | 0.758 | 1.0165 |  |
| png_high | 1.000 | 1.0000 |  |
| zenpng_e11_st | 1.198 | 1.0405 | lodepng, png_high |
| zenpng_e12_st | 1.871 | 1.0388 | lodepng, png_high |
| zenpng_e13_st | 3.077 | 1.0055 | png_high |

### enc by content: median time / png_high time, median size / png_high size

**document** (n=12)

| arm | time | size | dominated by |
|---|---|---|---|
| zune | 0.016 | 5.4439 |  |
| png_fast | 0.039 | 1.2275 |  |
| zenpng_e2_st | 0.086 | 1.1725 |  |
| zenpng_e1_st | 0.091 | 1.1441 |  |
| zenpng_e3_st | 0.120 | 1.1206 |  |
| zenpng_e4_st | 0.179 | 1.1110 |  |
| zenpng_e5_st | 0.207 | 1.0966 |  |
| zenpng_e6_st | 0.231 | 1.0870 |  |
| png_balanced | 0.242 | 1.0714 |  |
| zenpng_e7_st | 0.247 | 1.0803 | png_balanced |
| zenpng_e8_st | 0.259 | 1.0743 | png_balanced |
| lodepng | 0.329 | 1.0662 |  |
| zenpng_e9_st | 0.354 | 1.0348 |  |
| zenpng_e10_st | 0.729 | 1.0348 |  |
| png_high | 1.000 | 1.0000 |  |
| zenpng_e11_st | 1.127 | 1.0642 | png_high |
| zenpng_e12_st | 1.828 | 1.0440 | png_high |
| zenpng_e13_st | 3.095 | 0.9992 |  |

**lineart** (n=8)

| arm | time | size | dominated by |
|---|---|---|---|
| zune | 0.018 | 7.5150 |  |
| png_fast | 0.038 | 1.6021 |  |
| zenpng_e1_st | 0.087 | 1.2015 |  |
| zenpng_e2_st | 0.098 | 1.1475 |  |
| zenpng_e3_st | 0.132 | 1.1214 |  |
| png_balanced | 0.187 | 1.0874 |  |
| zenpng_e4_st | 0.192 | 1.1222 | png_balanced |
| zenpng_e5_st | 0.239 | 1.1092 | png_balanced |
| zenpng_e6_st | 0.253 | 1.1004 | png_balanced |
| zenpng_e7_st | 0.277 | 1.0785 |  |
| lodepng | 0.280 | 1.0511 |  |
| zenpng_e8_st | 0.325 | 1.0652 | lodepng |
| zenpng_e9_st | 0.352 | 1.0432 |  |
| zenpng_e10_st | 0.534 | 1.0432 |  |
| zenpng_e11_st | 0.757 | 1.0548 | lodepng |
| png_high | 1.000 | 1.0000 |  |
| zenpng_e12_st | 1.063 | 1.0549 | lodepng, png_high |
| zenpng_e13_st | 1.688 | 1.0385 | png_high |

**mixed** (n=6)

| arm | time | size | dominated by |
|---|---|---|---|
| zune | 0.017 | 1.5596 |  |
| png_fast | 0.055 | 1.1160 |  |
| zenpng_e2_st | 0.092 | 1.0682 |  |
| zenpng_e3_st | 0.099 | 1.0404 |  |
| zenpng_e1_st | 0.171 | 1.0428 |  |
| zenpng_e4_st | 0.176 | 1.0380 |  |
| zenpng_e5_st | 0.198 | 1.0229 |  |
| zenpng_e6_st | 0.210 | 1.0169 |  |
| zenpng_e7_st | 0.309 | 1.0124 |  |
| zenpng_e8_st | 0.402 | 1.0106 |  |
| png_balanced | 0.459 | 1.0120 |  |
| zenpng_e9_st | 0.513 | 1.0068 |  |
| lodepng | 0.592 | 1.0056 |  |
| png_high | 1.000 | 1.0000 |  |
| zenpng_e10_st | 1.300 | 1.0067 | lodepng, png_high |
| zenpng_e11_st | 1.636 | 1.0101 | lodepng, png_high |
| zenpng_e12_st | 2.826 | 1.0100 | lodepng, png_high |
| zenpng_e13_st | 4.998 | 1.0059 | lodepng, png_high |

**photo** (n=18)

| arm | time | size | dominated by |
|---|---|---|---|
| zune | 0.008 | 2.4466 |  |
| png_fast | 0.027 | 1.1262 |  |
| zenpng_e2_st | 0.045 | 1.1193 |  |
| zenpng_e1_st | 0.074 | 1.0880 |  |
| zenpng_e3_st | 0.080 | 1.0875 |  |
| zenpng_e4_st | 0.108 | 1.0623 |  |
| zenpng_e5_st | 0.133 | 1.0440 |  |
| zenpng_e6_st | 0.146 | 1.0311 |  |
| zenpng_e7_st | 0.180 | 1.0207 |  |
| zenpng_e8_st | 0.219 | 1.0157 |  |
| png_balanced | 0.220 | 1.0160 |  |
| zenpng_e9_st | 0.307 | 1.0091 |  |
| lodepng | 0.375 | 0.9987 |  |
| zenpng_e10_st | 0.639 | 1.0045 | lodepng |
| zenpng_e11_st | 0.750 | 1.0144 | lodepng |
| png_high | 1.000 | 1.0000 |  |
| zenpng_e12_st | 1.201 | 1.0144 | lodepng, png_high |
| zenpng_e13_st | 2.269 | 1.0007 | lodepng, png_high |

**screen** (n=6)

| arm | time | size | dominated by |
|---|---|---|---|
| zune | 0.020 | 7.2482 |  |
| png_fast | 0.044 | 1.5943 |  |
| zenpng_e1_st | 0.085 | 1.2671 |  |
| zenpng_e3_st | 0.132 | 1.2745 |  |
| zenpng_e2_st | 0.146 | 1.4945 |  |
| png_balanced | 0.218 | 1.1128 |  |
| zenpng_e4_st | 0.226 | 1.2501 | png_balanced |
| zenpng_e5_st | 0.253 | 1.2200 | png_balanced |
| zenpng_e6_st | 0.258 | 1.1947 | png_balanced |
| zenpng_e7_st | 0.278 | 1.1455 | png_balanced |
| zenpng_e8_st | 0.301 | 1.1174 | png_balanced |
| lodepng | 0.369 | 1.1017 |  |
| zenpng_e9_st | 0.386 | 1.0452 |  |
| zenpng_e10_st | 0.773 | 1.0452 |  |
| png_high | 1.000 | 1.0000 |  |
| zenpng_e11_st | 1.288 | 1.1032 | lodepng, png_high |
| zenpng_e12_st | 2.008 | 1.1032 | lodepng, png_high |
| zenpng_e13_st | 3.138 | 1.0205 | png_high |

