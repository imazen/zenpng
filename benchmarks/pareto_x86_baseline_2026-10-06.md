# benches/pareto.rs full run, x86 i265 (Core Ultra 7 265K), taskset -c 0-7 (P-cores), nice 19, run-heavy 16G (peak RSS 1.67 GiB, 3189 s).
# BASELINE before this session's encoder changes: zenpng 8bd0e2bc8f661ef76b12f4d8a48c73d9cb15a76e (release encodes still decompressed every candidate; no MT strips, no decode pipeline, old ladder).
# zenflate a6c5776cafbdd2688ba47be7479a620bdade1a27 dirty=3 (path dep; its uncommitted inflate WIP was compiled in).
# Inputs: scripts/vs_png_inputs.sh, SIZES="64 256 1024 4096" RGBA_SIZES="256 1024" (96 PNGs; only 5 sources reach 4096). The zenflate session's runs overlapped core 3 for the first minutes (log 11:31).
# Raw: i265 ~/tmp/pareto_full.{log,zb}, ~/tmp/pareto_x86_baseline.tsv (172 KB, not committed).

## enc

| arm | n | ms = a + b·MP (a ms, b ms/MP) | bytes = a + b·MP (enc) |
|---|---|---|---|
| lodepng | 75 | -4.797 + 72.425·MP | -108379 + 1349926·MP |
| png_balanced | 75 | -2.835 + 42.207·MP | -109003 + 1367184·MP |
| png_fast | 75 | -0.415 + 4.924·MP | -126947 + 1617495·MP |
| png_high | 75 | -15.376 + 241.953·MP | -109526 + 1338797·MP |
| zenpng_e13_mt | 75 | -2.705 + 150.889·MP | -107662 + 1336294·MP |
| zenpng_e13_st | 75 | -31.105 + 524.476·MP | -107662 + 1336294·MP |
| zenpng_e19_mt | 70 | -10.178 + 1275.174·MP | 19184 + 817973·MP |
| zenpng_e19_st | 70 | -25.467 + 2428.360·MP | 19184 + 817973·MP |
| zenpng_e1_st | 75 | -1.255 + 18.703·MP | -120109 + 1481878·MP |
| zenpng_e2_st | 75 | -3.232 + 72.545·MP | -115657 + 1449693·MP |
| zenpng_e3_st | 75 | -4.625 + 115.206·MP | -115955 + 1449501·MP |
| zenpng_e5_st | 75 | -5.393 + 127.727·MP | -113797 + 1435600·MP |
| zenpng_e7_idot8_mt | 75 | 1.473 + 48.206·MP | -113873 + 1435967·MP |
| zenpng_e7_mt | 75 | 1.056 + 39.844·MP | -113900 + 1435583·MP |
| zenpng_e7_st | 75 | -5.753 + 128.436·MP | -113900 + 1435583·MP |
| zenpng_e9_st | 75 | -14.017 + 251.902·MP | -110287 + 1385739·MP |
| zune | 75 | -0.645 + 4.958·MP | 60872 + 3003469·MP |

### enc by size_class: median time / png_high time, median size / png_high size

**large** (n=5)

| arm | time | size | dominated by |
|---|---|---|---|
| png_fast | 0.019 | 1.2078 |  |
| zune | 0.020 | 2.3079 |  |
| zenpng_e1_st | 0.074 | 1.1192 |  |
| zenpng_e7_mt | 0.153 | 1.0738 |  |
| png_balanced | 0.177 | 1.0205 |  |
| zenpng_e7_idot8_mt | 0.188 | 1.0743 | png_balanced |
| zenpng_e2_st | 0.277 | 1.0902 | png_balanced |
| lodepng | 0.337 | 1.0044 |  |
| zenpng_e3_st | 0.445 | 1.0896 | lodepng, png_balanced |
| zenpng_e5_st | 0.491 | 1.0738 | lodepng, png_balanced |
| zenpng_e7_st | 0.494 | 1.0738 | lodepng, png_balanced |
| zenpng_e13_mt | 0.592 | 1.0019 |  |
| png_high | 1.000 | 1.0000 |  |
| zenpng_e9_st | 1.012 | 1.0356 | lodepng, png_balanced, png_high |
| zenpng_e13_st | 2.065 | 1.0019 | png_high |

**medium** (n=25)

| arm | time | size | dominated by |
|---|---|---|---|
| zune | 0.018 | 3.7228 |  |
| png_fast | 0.023 | 1.2117 |  |
| zenpng_e1_st | 0.104 | 1.1242 |  |
| png_balanced | 0.158 | 1.0309 |  |
| lodepng | 0.304 | 1.0142 |  |
| zenpng_e7_mt | 0.315 | 1.0833 | lodepng, png_balanced |
| zenpng_e7_idot8_mt | 0.404 | 1.0834 | lodepng, png_balanced |
| zenpng_e2_st | 0.501 | 1.0917 | lodepng, png_balanced |
| zenpng_e3_st | 0.730 | 1.0917 | lodepng, png_balanced |
| zenpng_e7_st | 0.830 | 1.0833 | lodepng, png_balanced |
| zenpng_e5_st | 0.852 | 1.0833 | lodepng, png_balanced |
| png_high | 1.000 | 1.0000 |  |
| zenpng_e13_mt | 1.086 | 1.0052 | png_high |
| zenpng_e9_st | 1.662 | 1.0275 | lodepng, png_high |
| zenpng_e13_st | 3.148 | 1.0052 | png_high |
| zenpng_e19_mt | 7.662 | 0.9458 |  |
| zenpng_e19_st | 14.404 | 0.9458 |  |

**small** (n=25)

| arm | time | size | dominated by |
|---|---|---|---|
| zune | 0.018 | 3.0251 |  |
| png_fast | 0.035 | 1.1826 |  |
| zenpng_e1_st | 0.118 | 1.1114 |  |
| png_balanced | 0.235 | 1.0463 |  |
| lodepng | 0.372 | 1.0234 |  |
| zenpng_e2_st | 0.517 | 1.0876 | lodepng, png_balanced |
| zenpng_e7_mt | 0.552 | 1.0837 | lodepng, png_balanced |
| zenpng_e7_idot8_mt | 0.567 | 1.0837 | lodepng, png_balanced |
| zenpng_e3_st | 0.886 | 1.0866 | lodepng, png_balanced |
| png_high | 1.000 | 1.0000 |  |
| zenpng_e5_st | 1.000 | 1.0837 | lodepng, png_balanced, png_high |
| zenpng_e7_st | 1.004 | 1.0837 | lodepng, png_balanced, png_high |
| zenpng_e13_mt | 1.425 | 0.9980 |  |
| zenpng_e9_st | 1.844 | 1.0302 | lodepng, png_high |
| zenpng_e13_st | 3.340 | 0.9980 |  |
| zenpng_e19_mt | 10.899 | 0.9488 |  |
| zenpng_e19_st | 18.901 | 0.9488 |  |

**tiny** (n=20)

| arm | time | size | dominated by |
|---|---|---|---|
| zune | 0.025 | 2.1312 |  |
| png_fast | 0.054 | 1.1223 |  |
| zenpng_e1_st | 0.361 | 1.0619 |  |
| png_balanced | 0.550 | 1.0183 |  |
| lodepng | 0.703 | 1.0059 |  |
| png_high | 1.000 | 1.0000 |  |
| zenpng_e2_st | 1.299 | 1.0545 | lodepng, png_balanced, png_high |
| zenpng_e7_mt | 2.343 | 1.0393 | lodepng, png_balanced, png_high |
| zenpng_e7_idot8_mt | 2.374 | 1.0393 | lodepng, png_balanced, png_high |
| zenpng_e3_st | 2.629 | 1.0486 | lodepng, png_balanced, png_high |
| zenpng_e5_st | 2.822 | 1.0394 | lodepng, png_balanced, png_high |
| zenpng_e7_st | 2.835 | 1.0393 | lodepng, png_balanced, png_high |
| zenpng_e9_st | 5.178 | 1.0115 | lodepng, png_high |
| zenpng_e13_mt | 6.462 | 0.9983 |  |
| zenpng_e13_st | 8.840 | 0.9983 |  |
| zenpng_e19_mt | 29.783 | 0.9742 |  |
| zenpng_e19_st | 38.408 | 0.9742 |  |

### enc by content: median time / png_high time, median size / png_high size

**document** (n=17)

| arm | time | size | dominated by |
|---|---|---|---|
| zune | 0.022 | 2.8998 |  |
| png_fast | 0.032 | 1.2168 |  |
| zenpng_e1_st | 0.121 | 1.1242 |  |
| png_balanced | 0.249 | 1.0522 |  |
| lodepng | 0.438 | 1.0471 |  |
| zenpng_e2_st | 0.546 | 1.0929 | lodepng, png_balanced |
| zenpng_e7_mt | 0.552 | 1.0738 | lodepng, png_balanced |
| zenpng_e7_idot8_mt | 0.612 | 1.0743 | lodepng, png_balanced |
| png_high | 1.000 | 1.0000 |  |
| zenpng_e3_st | 1.070 | 1.0917 | lodepng, png_balanced, png_high |
| zenpng_e5_st | 1.127 | 1.0738 | lodepng, png_balanced, png_high |
| zenpng_e7_st | 1.133 | 1.0738 | lodepng, png_balanced, png_high |
| zenpng_e13_mt | 1.303 | 0.9938 |  |
| zenpng_e9_st | 1.917 | 1.0212 | png_high |
| zenpng_e13_st | 3.486 | 0.9938 |  |
| zenpng_e19_mt | 10.743 | 0.9511 |  |
| zenpng_e19_st | 19.702 | 0.9511 |  |

**lineart** (n=11)

| arm | time | size | dominated by |
|---|---|---|---|
| zune | 0.021 | 6.6755 |  |
| png_fast | 0.033 | 1.4790 |  |
| zenpng_e1_st | 0.185 | 1.1262 |  |
| png_balanced | 0.202 | 1.0732 |  |
| lodepng | 0.324 | 1.0436 |  |
| zenpng_e2_st | 0.478 | 1.1069 | lodepng, png_balanced |
| zenpng_e7_mt | 0.537 | 1.0945 | lodepng, png_balanced |
| zenpng_e7_idot8_mt | 0.632 | 1.0945 | lodepng, png_balanced |
| zenpng_e3_st | 0.830 | 1.1015 | lodepng, png_balanced |
| zenpng_e5_st | 0.872 | 1.0952 | lodepng, png_balanced |
| zenpng_e7_st | 0.880 | 1.0945 | lodepng, png_balanced |
| png_high | 1.000 | 1.0000 |  |
| zenpng_e13_mt | 1.234 | 1.0020 | png_high |
| zenpng_e9_st | 1.469 | 1.0151 | png_high |
| zenpng_e13_st | 2.709 | 1.0020 | png_high |
| zenpng_e19_mt | 11.996 | 0.9422 |  |
| zenpng_e19_st | 20.074 | 0.9422 |  |

**mixed** (n=11)

| arm | time | size | dominated by |
|---|---|---|---|
| zune | 0.028 | 1.5478 |  |
| png_fast | 0.049 | 1.1107 |  |
| zenpng_e1_st | 0.257 | 1.0436 |  |
| png_balanced | 0.462 | 1.0102 |  |
| lodepng | 0.607 | 1.0020 |  |
| zenpng_e7_mt | 0.970 | 1.0354 | lodepng, png_balanced |
| zenpng_e7_idot8_mt | 0.986 | 1.0361 | lodepng, png_balanced |
| png_high | 1.000 | 1.0000 |  |
| zenpng_e2_st | 1.274 | 1.0432 | lodepng, png_balanced, png_high |
| zenpng_e3_st | 1.915 | 1.0403 | lodepng, png_balanced, png_high |
| zenpng_e5_st | 2.115 | 1.0355 | lodepng, png_balanced, png_high |
| zenpng_e7_st | 2.124 | 1.0354 | lodepng, png_balanced, png_high |
| zenpng_e13_mt | 2.793 | 1.0052 | lodepng, png_high |
| zenpng_e9_st | 4.080 | 1.0204 | lodepng, png_balanced, png_high |
| zenpng_e13_st | 7.416 | 1.0052 | lodepng, png_high |
| zenpng_e19_mt | 14.128 | 0.9710 |  |
| zenpng_e19_st | 25.935 | 0.9710 |  |

**photo** (n=28)

| arm | time | size | dominated by |
|---|---|---|---|
| zune | 0.018 | 2.3168 |  |
| png_fast | 0.025 | 1.1197 |  |
| zenpng_e1_st | 0.113 | 1.0840 |  |
| png_balanced | 0.246 | 1.0122 |  |
| zenpng_e7_mt | 0.383 | 1.0631 | png_balanced |
| lodepng | 0.405 | 0.9981 |  |
| zenpng_e7_idot8_mt | 0.413 | 1.0635 | lodepng, png_balanced |
| zenpng_e2_st | 0.509 | 1.0703 | lodepng, png_balanced |
| zenpng_e3_st | 0.786 | 1.0703 | lodepng, png_balanced |
| zenpng_e5_st | 0.876 | 1.0631 | lodepng, png_balanced |
| zenpng_e7_st | 0.879 | 1.0631 | lodepng, png_balanced |
| png_high | 1.000 | 1.0000 |  |
| zenpng_e13_mt | 1.173 | 0.9994 | lodepng |
| zenpng_e9_st | 1.672 | 1.0265 | lodepng, png_balanced, png_high |
| zenpng_e13_st | 3.291 | 0.9994 | lodepng |
| zenpng_e19_mt | 9.501 | 0.9586 |  |
| zenpng_e19_st | 17.634 | 0.9586 |  |

**screen** (n=8)

| arm | time | size | dominated by |
|---|---|---|---|
| zune | 0.026 | 6.3333 |  |
| png_fast | 0.037 | 1.4393 |  |
| zenpng_e1_st | 0.138 | 1.1862 |  |
| png_balanced | 0.215 | 1.0955 |  |
| lodepng | 0.379 | 1.0813 |  |
| zenpng_e2_st | 0.586 | 1.1706 | lodepng, png_balanced |
| zenpng_e7_mt | 0.748 | 1.1612 | lodepng, png_balanced |
| zenpng_e7_idot8_mt | 0.771 | 1.1612 | lodepng, png_balanced |
| png_high | 1.000 | 1.0000 |  |
| zenpng_e3_st | 1.180 | 1.1700 | lodepng, png_balanced, png_high |
| zenpng_e5_st | 1.256 | 1.1618 | lodepng, png_balanced, png_high |
| zenpng_e7_st | 1.267 | 1.1612 | lodepng, png_balanced, png_high |
| zenpng_e13_mt | 1.652 | 1.0175 | png_high |
| zenpng_e9_st | 2.075 | 1.0575 | png_high |
| zenpng_e13_st | 3.719 | 1.0175 | png_high |
| zenpng_e19_mt | 14.068 | 0.9370 |  |
| zenpng_e19_st | 24.733 | 0.9370 |  |

## dec

| arm | n | ms = a + b·MP (a ms, b ms/MP) | bytes = a + b·MP (enc) |
|---|---|---|---|
| lodepng | 96 | -0.833 + 6.313·MP |  |
| png | 96 | -0.712 + 5.128·MP |  |
| zenpng_mt | 96 | -0.781 + 5.501·MP |  |
| zenpng_st | 96 | -0.778 + 5.494·MP |  |
| zune | 96 | -1.686 + 8.600·MP |  |

### dec by size_class: median time / png time

**large** (n=5)

| arm | time |
|---|---|
| png | 1.000 |
| zenpng_st | 1.072 |
| zenpng_mt | 1.072 |
| lodepng | 1.258 |
| zune | 1.697 |

**medium** (n=46)

| arm | time |
|---|---|
| png | 1.000 |
| zenpng_st | 1.058 |
| zenpng_mt | 1.060 |
| lodepng | 1.223 |
| zune | 1.227 |

**small** (n=25)

| arm | time |
|---|---|
| png | 1.000 |
| zenpng_mt | 1.040 |
| zenpng_st | 1.042 |
| lodepng | 1.211 |
| zune | 1.240 |

**tiny** (n=20)

| arm | time |
|---|---|
| png | 1.000 |
| zenpng_st | 1.082 |
| zenpng_mt | 1.082 |
| lodepng | 1.244 |
| zune | 1.251 |

### dec by content: median time / png time

**document** (n=25)

| arm | time |
|---|---|
| png | 1.000 |
| zenpng_mt | 1.050 |
| zenpng_st | 1.052 |
| lodepng | 1.201 |
| zune | 1.236 |

**lineart** (n=16)

| arm | time |
|---|---|
| png | 1.000 |
| zenpng_mt | 1.042 |
| zenpng_st | 1.044 |
| lodepng | 1.120 |
| zune | 1.139 |

**mixed** (n=11)

| arm | time |
|---|---|
| png | 1.000 |
| zenpng_mt | 1.078 |
| zenpng_st | 1.079 |
| lodepng | 1.329 |
| zune | 1.429 |

**photo** (n=32)

| arm | time |
|---|---|
| png | 1.000 |
| zenpng_st | 1.075 |
| zenpng_mt | 1.078 |
| lodepng | 1.316 |
| zune | 1.319 |

**screen** (n=12)

| arm | time |
|---|---|
| png | 1.000 |
| zenpng_mt | 1.061 |
| zenpng_st | 1.063 |
| lodepng | 1.116 |
| zune | 1.170 |

## dec_idot

| arm | n | ms = a + b·MP (a ms, b ms/MP) | bytes = a + b·MP (enc) |
|---|---|---|---|
| lodepng | 75 | -0.356 + 6.113·MP |  |
| png | 75 | -0.352 + 5.417·MP |  |
| zenpng_mt | 75 | 0.536 + 0.989·MP |  |
| zenpng_st | 75 | -0.324 + 5.452·MP |  |
| zune | 75 | -1.259 + 9.772·MP |  |

### dec_idot by size_class: median time / png time

**large** (n=5)

| arm | time |
|---|---|
| zenpng_mt | 0.187 |
| png | 1.000 |
| zenpng_st | 1.024 |
| lodepng | 1.152 |
| zune | 1.936 |

**medium** (n=25)

| arm | time |
|---|---|
| zenpng_mt | 0.655 |
| png | 1.000 |
| zenpng_st | 1.036 |
| lodepng | 1.132 |
| zune | 1.157 |

**small** (n=25)

| arm | time |
|---|---|
| zenpng_mt | 0.999 |
| png | 1.000 |
| zenpng_st | 1.002 |
| zune | 1.137 |
| lodepng | 1.146 |

**tiny** (n=20)

| arm | time |
|---|---|
| png | 1.000 |
| zenpng_mt | 1.057 |
| zenpng_st | 1.058 |
| lodepng | 1.148 |
| zune | 1.240 |

### dec_idot by content: median time / png time

**document** (n=17)

| arm | time |
|---|---|
| zenpng_mt | 0.964 |
| png | 1.000 |
| zenpng_st | 1.021 |
| lodepng | 1.117 |
| zune | 1.137 |

**lineart** (n=11)

| arm | time |
|---|---|
| png | 1.000 |
| zenpng_mt | 1.017 |
| zenpng_st | 1.046 |
| lodepng | 1.047 |
| zune | 1.062 |

**mixed** (n=11)

| arm | time |
|---|---|
| png | 1.000 |
| zenpng_mt | 1.034 |
| zenpng_st | 1.035 |
| lodepng | 1.231 |
| zune | 1.335 |

**photo** (n=28)

| arm | time |
|---|---|
| zenpng_mt | 0.973 |
| png | 1.000 |
| zenpng_st | 1.024 |
| lodepng | 1.179 |
| zune | 1.352 |

**screen** (n=8)

| arm | time |
|---|---|
| png | 1.000 |
| zenpng_mt | 1.050 |
| zenpng_st | 1.068 |
| zune | 1.072 |
| lodepng | 1.079 |

