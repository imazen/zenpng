# benches/pareto.rs --group=dec, x86 i265 (Core Ultra 7 265K) P-cores (taskset -c 0-7), nice 19, zenbench lock. 2026-10-07.
# zenpng 0f75a1b0bf2ca1bfed8bd92c11e534e4294e42b0 (before f9ab5a1's decoder box/slack change). zenflate png-mode 66ead6f739cb3be4ba27ad41230c9da5938ec719 (clean; 32-byte match copy, v3 streaming loop, v4 one-shot).
# 97 inputs from scripts/vs_png_inputs.sh SIZES="64 256 1024 4096" RGBA_SIZES="256 1024" (~/tmp/pareto_in, same set as pareto_x86_decode_2026-10-06.md). zenpng_mt = max_threads(0) (pipeline / iDOT); others single-threaded.
# Since 2026-10-06 (zenflate 8f886ab): zenpng_st median vs image-png went tiny 1.11 -> 1.063, overall 1.07 -> ~1.0 (large 1.017, medium 1.004, small 0.996).
# Command: ZENPNG_PARETO_DIR=~/tmp/pareto_in nice -n19 taskset -c 0-7 <pareto bench> --bench --group=dec; report scripts/pareto_report.py.

## dec

| arm | n | ms = a + b·MP (a ms, b ms/MP; relative-error fit) | bytes = a + b·MP (enc) |
|---|---|---|---|
| lodepng | 96 | 0.0128 + 0.942·MP |  |
| png | 96 | 0.0095 + 1.302·MP |  |
| zenpng_mt | 96 | 0.0128 + 0.330·MP |  |
| zenpng_st | 96 | 0.0129 + 0.312·MP |  |
| zune | 96 | 0.0135 + 0.634·MP |  |

### dec by size_class: median time / png time

**large** (n=5)

| arm | time |
|---|---|
| zenpng_mt | 0.623 |
| png | 1.000 |
| zenpng_st | 1.017 |
| lodepng | 1.224 |
| zune | 1.554 |

**medium** (n=46)

| arm | time |
|---|---|
| zenpng_mt | 0.913 |
| png | 1.000 |
| zenpng_st | 1.004 |
| lodepng | 1.221 |
| zune | 1.238 |

**small** (n=25)

| arm | time |
|---|---|
| zenpng_st | 0.996 |
| zenpng_mt | 0.997 |
| png | 1.000 |
| lodepng | 1.209 |
| zune | 1.227 |

**tiny** (n=20)

| arm | time |
|---|---|
| png | 1.000 |
| zenpng_mt | 1.061 |
| zenpng_st | 1.063 |
| zune | 1.216 |
| lodepng | 1.225 |

### dec by content: median time / png time

**document** (n=25)

| arm | time |
|---|---|
| zenpng_mt | 0.983 |
| png | 1.000 |
| zenpng_st | 1.011 |
| lodepng | 1.166 |
| zune | 1.208 |

**lineart** (n=16)

| arm | time |
|---|---|
| zenpng_mt | 0.937 |
| zenpng_st | 0.976 |
| png | 1.000 |
| lodepng | 1.117 |
| zune | 1.127 |

**mixed** (n=11)

| arm | time |
|---|---|
| png | 1.000 |
| zenpng_mt | 1.026 |
| zenpng_st | 1.050 |
| lodepng | 1.333 |
| zune | 1.396 |

**photo** (n=32)

| arm | time |
|---|---|
| zenpng_mt | 0.986 |
| png | 1.000 |
| zenpng_st | 1.004 |
| lodepng | 1.309 |
| zune | 1.314 |

**screen** (n=12)

| arm | time |
|---|---|
| zenpng_mt | 0.986 |
| zenpng_st | 0.993 |
| png | 1.000 |
| lodepng | 1.114 |
| zune | 1.163 |

## dec_idot

| arm | n | ms = a + b·MP (a ms, b ms/MP; relative-error fit) | bytes = a + b·MP (enc) |
|---|---|---|---|
| lodepng | 75 | 0.0024 + 3.779·MP |  |
| png | 75 | 0.0020 + 3.509·MP |  |
| zenpng_mt | 75 | 0.0061 + 2.175·MP |  |
| zenpng_st | 75 | 0.0021 + 3.493·MP |  |
| zune | 75 | 0.0018 + 4.236·MP |  |

### dec_idot by size_class: median time / png time

**large** (n=5)

| arm | time |
|---|---|
| zenpng_mt | 0.205 |
| zenpng_st | 0.991 |
| png | 1.000 |
| lodepng | 1.152 |
| zune | 1.280 |

**medium** (n=25)

| arm | time |
|---|---|
| zenpng_mt | 0.618 |
| zenpng_st | 0.993 |
| png | 1.000 |
| lodepng | 1.121 |
| zune | 1.157 |

**small** (n=25)

| arm | time |
|---|---|
| zenpng_mt | 0.968 |
| zenpng_st | 0.970 |
| png | 1.000 |
| zune | 1.100 |
| lodepng | 1.124 |

**tiny** (n=20)

| arm | time |
|---|---|
| zenpng_mt | 0.928 |
| zenpng_st | 0.930 |
| png | 1.000 |
| lodepng | 1.058 |
| zune | 1.152 |

### dec_idot by content: median time / png time

**document** (n=17)

| arm | time |
|---|---|
| zenpng_mt | 0.873 |
| zenpng_st | 0.961 |
| png | 1.000 |
| lodepng | 1.051 |
| zune | 1.084 |

**lineart** (n=11)

| arm | time |
|---|---|
| zenpng_mt | 0.963 |
| zenpng_st | 0.997 |
| png | 1.000 |
| lodepng | 1.101 |
| zune | 1.111 |

**mixed** (n=11)

| arm | time |
|---|---|
| zenpng_mt | 0.850 |
| png | 1.000 |
| zenpng_st | 1.009 |
| lodepng | 1.172 |
| zune | 1.215 |

**photo** (n=28)

| arm | time |
|---|---|
| zenpng_mt | 0.924 |
| zenpng_st | 0.973 |
| png | 1.000 |
| lodepng | 1.160 |
| zune | 1.177 |

**screen** (n=8)

| arm | time |
|---|---|
| zenpng_mt | 0.931 |
| zenpng_st | 0.993 |
| png | 1.000 |
| lodepng | 1.040 |
| zune | 1.119 |

