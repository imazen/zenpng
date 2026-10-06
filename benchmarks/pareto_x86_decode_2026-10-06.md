# benches/pareto.rs --group=dec, x86 i265 P-cores (taskset -c 0-7), nice 19. zenpng 42cf38659de69aa2ff0f3c7037097f51337e3770 (two-thread decode pipeline from 512 KiB, ARM min-select Paeth, color index).
# zenflate 8f886abdd5fd430a45814e1ca8e7142b551a791c dirty  M src/compress/mod.rs 
# 96 inputs from scripts/vs_png_inputs.sh SIZES="64 256 1024 4096" RGBA_SIZES="256 1024". zenpng_mt = max_threads(0) (pipeline / iDOT); others single-threaded.
# Raw: i265 ~/tmp/pareto_x86_dec.{log,zb,tsv}.

## dec

| arm | n | ms = a + b·MP (a ms, b ms/MP; relative-error fit) | bytes = a + b·MP (enc) |
|---|---|---|---|
| lodepng | 96 | 0.0128 + 0.943·MP |  |
| png | 96 | 0.0095 + 1.299·MP |  |
| zenpng_mt | 96 | 0.0134 + 0.370·MP |  |
| zenpng_st | 96 | 0.0135 + 0.357·MP |  |
| zune | 96 | 0.0136 + 0.637·MP |  |

### dec by size_class: median time / png time

**large** (n=5)

| arm | time |
|---|---|
| zenpng_mt | 0.660 |
| png | 1.000 |
| zenpng_st | 1.067 |
| lodepng | 1.228 |
| zune | 1.535 |

**medium** (n=46)

| arm | time |
|---|---|
| zenpng_mt | 0.906 |
| png | 1.000 |
| zenpng_st | 1.066 |
| zune | 1.218 |
| lodepng | 1.225 |

**small** (n=25)

| arm | time |
|---|---|
| png | 1.000 |
| zenpng_mt | 1.064 |
| zenpng_st | 1.067 |
| lodepng | 1.212 |
| zune | 1.223 |

**tiny** (n=20)

| arm | time |
|---|---|
| png | 1.000 |
| zenpng_mt | 1.113 |
| zenpng_st | 1.114 |
| zune | 1.232 |
| lodepng | 1.234 |

### dec by content: median time / png time

**document** (n=25)

| arm | time |
|---|---|
| png | 1.000 |
| zenpng_mt | 1.033 |
| zenpng_st | 1.067 |
| lodepng | 1.192 |
| zune | 1.216 |

**lineart** (n=16)

| arm | time |
|---|---|
| zenpng_mt | 0.985 |
| png | 1.000 |
| zenpng_st | 1.029 |
| lodepng | 1.125 |
| zune | 1.135 |

**mixed** (n=11)

| arm | time |
|---|---|
| png | 1.000 |
| zenpng_mt | 1.060 |
| zenpng_st | 1.086 |
| lodepng | 1.348 |
| zune | 1.416 |

**photo** (n=32)

| arm | time |
|---|---|
| png | 1.000 |
| zenpng_mt | 1.061 |
| zenpng_st | 1.084 |
| zune | 1.306 |
| lodepng | 1.311 |

**screen** (n=12)

| arm | time |
|---|---|
| png | 1.000 |
| zenpng_mt | 1.032 |
| zenpng_st | 1.037 |
| lodepng | 1.113 |
| zune | 1.177 |

## dec_idot

| arm | n | ms = a + b·MP (a ms, b ms/MP; relative-error fit) | bytes = a + b·MP (enc) |
|---|---|---|---|
| lodepng | 75 | 0.0078 + 1.382·MP |  |
| png | 75 | 0.0066 + 1.735·MP |  |
| zenpng_mt | 75 | 0.0071 + 1.704·MP |  |
| zenpng_st | 75 | 0.0067 + 1.843·MP |  |
| zune | 75 | 0.0079 + 1.756·MP |  |

### dec_idot by size_class: median time / png time

**large** (n=5)

| arm | time |
|---|---|
| zenpng_mt | 0.219 |
| png | 1.000 |
| zenpng_st | 1.038 |
| lodepng | 1.167 |
| zune | 1.732 |

**medium** (n=25)

| arm | time |
|---|---|
| zenpng_mt | 0.648 |
| png | 1.000 |
| zenpng_st | 1.053 |
| lodepng | 1.134 |
| zune | 1.160 |

**small** (n=25)

| arm | time |
|---|---|
| png | 1.000 |
| zenpng_mt | 1.016 |
| zenpng_st | 1.020 |
| lodepng | 1.145 |
| zune | 1.149 |

**tiny** (n=20)

| arm | time |
|---|---|
| png | 1.000 |
| zenpng_mt | 1.050 |
| zenpng_st | 1.052 |
| lodepng | 1.152 |
| zune | 1.214 |

### dec_idot by content: median time / png time

**document** (n=17)

| arm | time |
|---|---|
| zenpng_mt | 0.999 |
| png | 1.000 |
| zenpng_st | 1.020 |
| lodepng | 1.115 |
| zune | 1.146 |

**lineart** (n=11)

| arm | time |
|---|---|
| png | 1.000 |
| zenpng_mt | 1.007 |
| zenpng_st | 1.053 |
| lodepng | 1.059 |
| zune | 1.064 |

**mixed** (n=11)

| arm | time |
|---|---|
| png | 1.000 |
| zenpng_mt | 1.045 |
| zenpng_st | 1.049 |
| lodepng | 1.234 |
| zune | 1.345 |

**photo** (n=28)

| arm | time |
|---|---|
| zenpng_mt | 0.998 |
| png | 1.000 |
| zenpng_st | 1.029 |
| lodepng | 1.190 |
| zune | 1.323 |

**screen** (n=8)

| arm | time |
|---|---|
| png | 1.000 |
| zenpng_mt | 1.021 |
| zenpng_st | 1.051 |
| zune | 1.070 |
| lodepng | 1.089 |

