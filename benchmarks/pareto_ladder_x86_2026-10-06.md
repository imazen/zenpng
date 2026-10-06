# x86 encode Pareto: effort ladder with the None/Paeth/MinSum screen (2026-10-06)

- zenpng: the "feat: effort ladder on zenflate png() levels" commit (bench built from working copy c2d2930417bf)
- zenflate: png-mode 8b8cf0f (clean)
- host: i265 (Core Ultra 7 265K), P-cores 0-7, nice 19, zenbench lock
- inputs: `scripts/vs_png_inputs.sh` output, 70 RGB8/RGBA8 imazen-26 renditions at 64/256/1024 px (`~/tmp/pareto_in_le1024`)
- command: `ZENPNG_PARETO_DIR=... ZENPNG_PARETO_NO_IDOT=1 ZENPNG_PARETO_EFFORTS=1..19 ZENPNG_PARETO_MT_EFFORTS=1,7,13 nice -n19 taskset -c 0-7 <pareto bench> --bench --group=enc`, report `scripts/pareto_report.py`
- times are zenbench means; "time"/"size" in the tables are medians of per-image ratios to image-png main's `High`

## Per content class at the preset rungs (geomean vs png High)

"Draft" columns: the first draft of this ladder (Paeth+MinSum screened at png(1) at every rung), same harness and inputs, run earlier the same day.

| class (n) | rung | size | time | draft size | draft time |
|---|---|---|---|---|---|
| photo (26) | Fast e7 | 1.0070 | 0.64 | 1.0070 | 0.64 |
| photo (26) | Balanced e13 | 0.9690 | 3.76 | 0.9689 | 3.18 |
| photo (26) | Thorough e17 | 0.9570 | 16.21 | 0.9561 | 21.22 |
| photo (26) | High e19 | 0.9538 | 43.66 | 0.9553 | 48.76 |
| screen (8) | Fast e7 | 1.0350 | 0.54 | 1.0350 | 0.55 |
| screen (8) | Balanced e13 | 0.9593 | 4.10 | 0.9638 | 4.28 |
| screen (8) | Thorough e17 | 0.9373 | 25.97 | 0.9433 | 31.83 |
| screen (8) | High e19 | 0.9323 | 69.62 | 0.9424 | 78.83 |
| lineart (11) | Fast e7 | 0.9292 | 0.47 | 0.9292 | 0.47 |
| lineart (11) | Balanced e13 | 0.8401 | 3.36 | 0.8768 | 3.53 |
| lineart (11) | Thorough e17 | 0.8214 | 19.15 | 0.8464 | 24.20 |
| lineart (11) | High e19 | 0.8211 | 51.15 | 0.8435 | 58.97 |
| document (16) | Fast e7 | 1.0347 | 0.62 | 1.0347 | 0.63 |
| document (16) | Balanced e13 | 0.9450 | 3.74 | 0.9774 | 3.64 |
| document (16) | Thorough e17 | 0.9261 | 19.91 | 0.9496 | 25.61 |
| document (16) | High e19 | 0.9232 | 54.88 | 0.9475 | 61.71 |
| mixed (9) | Fast e7 | 0.9911 | 0.85 | 0.9911 | 0.85 |
| mixed (9) | Balanced e13 | 0.9727 | 4.44 | 0.9727 | 3.22 |
| mixed (9) | Thorough e17 | 0.9700 | 17.95 | 0.9698 | 23.13 |
| mixed (9) | High e19 | 0.9691 | 47.77 | 0.9694 | 53.92 |

The draft's e13/e17 were different searches (Paeth+MinSum at png(24); heuristic top 3 + BF(3,1)), so rows compare presets, not identical configs. The shipped pre-rework ladder is in `pareto_x86_baseline_2026-10-06.md`.

## enc

| arm | n | ms = a + b·MP (a ms, b ms/MP; relative-error fit) | bytes = a + b·MP (enc) |
|---|---|---|---|
| lodepng | 70 | 0.0403 + 23.675·MP | 2122 + 353830·MP |
| png_balanced | 70 | 0.0582 + 11.693·MP | 2380 + 458250·MP |
| png_fast | 70 | 0.0019 + 2.372·MP | 2555 + 593928·MP |
| png_high | 70 | -0.0284 + 64.952·MP | 2341 + 381377·MP |
| zenpng_e10_st | 70 | 0.0860 + 53.086·MP | 2390 + 297394·MP |
| zenpng_e11_st | 70 | 0.0789 + 56.042·MP | 2391 + 296879·MP |
| zenpng_e12_st | 70 | 0.1493 + 72.362·MP | 2391 + 296546·MP |
| zenpng_e13_mt | 70 | 1.2871 + 100.575·MP | 2408 + 279693·MP |
| zenpng_e13_st | 70 | 0.3617 + 231.776·MP | 2411 + 278832·MP |
| zenpng_e14_st | 70 | 0.2430 + 277.336·MP | 2414 + 276768·MP |
| zenpng_e15_st | 70 | 0.3146 + 403.970·MP | 2405 + 273829·MP |
| zenpng_e16_st | 70 | 1.7843 + 916.390·MP | 2403 + 273760·MP |
| zenpng_e17_st | 70 | 1.7166 + 1348.898·MP | 2407 + 271650·MP |
| zenpng_e18_st | 70 | 3.0854 + 1942.917·MP | 2406 + 271640·MP |
| zenpng_e19_st | 70 | 3.8937 + 3720.889·MP | 2404 + 271466·MP |
| zenpng_e1_mt | 70 | 0.0222 + 2.263·MP | 2437 + 381367·MP |
| zenpng_e1_st | 70 | 0.0114 + 2.078·MP | 2442 + 379481·MP |
| zenpng_e2_st | 70 | 0.0184 + 9.230·MP | 2377 + 366498·MP |
| zenpng_e3_st | 70 | 0.0195 + 12.853·MP | 2362 + 354367·MP |
| zenpng_e4_st | 70 | 0.0274 + 13.708·MP | 2341 + 351984·MP |
| zenpng_e5_st | 70 | 0.0265 + 14.863·MP | 2334 + 349922·MP |
| zenpng_e6_st | 70 | 0.0570 + 30.909·MP | 2344 + 330128·MP |
| zenpng_e7_mt | 70 | 0.5019 + 14.688·MP | 2338 + 328087·MP |
| zenpng_e7_st | 70 | 0.0589 + 33.736·MP | 2337 + 328180·MP |
| zenpng_e8_st | 70 | 0.0688 + 45.028·MP | 2396 + 299082·MP |
| zenpng_e9_st | 70 | 0.0918 + 47.701·MP | 2392 + 298524·MP |
| zune | 70 | -0.0012 + 1.992·MP | -238 + 3132194·MP |

### enc by size_class: median time / png_high time, median size / png_high size

**medium** (n=25)

| arm | time | size | dominated by |
|---|---|---|---|
| zune | 0.016 | 3.7228 |  |
| zenpng_e1_mt | 0.017 | 1.1445 |  |
| zenpng_e1_st | 0.018 | 1.1516 |  |
| png_fast | 0.023 | 1.2117 |  |
| zenpng_e2_st | 0.073 | 1.1088 |  |
| zenpng_e3_st | 0.131 | 1.0923 |  |
| png_balanced | 0.157 | 1.0309 |  |
| zenpng_e4_st | 0.168 | 1.0750 | png_balanced |
| zenpng_e7_mt | 0.179 | 1.0214 |  |
| zenpng_e5_st | 0.199 | 1.0542 | png_balanced |
| lodepng | 0.303 | 1.0142 |  |
| zenpng_e6_st | 0.442 | 1.0357 | lodepng, png_balanced |
| zenpng_e7_st | 0.481 | 1.0194 | lodepng |
| zenpng_e8_st | 0.627 | 1.0081 |  |
| zenpng_e9_st | 0.661 | 1.0065 |  |
| zenpng_e10_st | 0.777 | 1.0039 |  |
| zenpng_e11_st | 0.839 | 1.0019 |  |
| zenpng_e13_mt | 0.977 | 0.9646 |  |
| png_high | 1.000 | 1.0000 |  |
| zenpng_e12_st | 1.078 | 1.0019 | png_high |
| zenpng_e13_st | 2.984 | 0.9622 |  |
| zenpng_e14_st | 3.050 | 0.9568 |  |
| zenpng_e15_st | 4.133 | 0.9514 |  |
| zenpng_e16_st | 8.197 | 0.9415 |  |
| zenpng_e17_st | 12.481 | 0.9415 |  |
| zenpng_e18_st | 16.667 | 0.9410 |  |
| zenpng_e19_st | 33.490 | 0.9371 |  |

**small** (n=25)

| arm | time | size | dominated by |
|---|---|---|---|
| zune | 0.018 | 3.0251 |  |
| zenpng_e1_st | 0.026 | 1.1393 |  |
| png_fast | 0.032 | 1.1826 |  |
| zenpng_e1_mt | 0.034 | 1.1393 |  |
| zenpng_e2_st | 0.102 | 1.1020 |  |
| zenpng_e3_st | 0.161 | 1.0769 |  |
| zenpng_e4_st | 0.183 | 1.0567 |  |
| zenpng_e5_st | 0.208 | 1.0557 |  |
| png_balanced | 0.228 | 1.0463 |  |
| lodepng | 0.370 | 1.0234 |  |
| zenpng_e6_st | 0.458 | 1.0277 | lodepng |
| zenpng_e7_st | 0.552 | 1.0214 |  |
| zenpng_e7_mt | 0.660 | 1.0214 |  |
| zenpng_e8_st | 0.752 | 1.0196 |  |
| zenpng_e9_st | 0.787 | 1.0189 |  |
| zenpng_e10_st | 0.879 | 1.0162 |  |
| zenpng_e11_st | 0.930 | 1.0162 |  |
| png_high | 1.000 | 1.0000 |  |
| zenpng_e12_st | 1.236 | 1.0066 | png_high |
| zenpng_e13_mt | 2.978 | 0.9607 |  |
| zenpng_e13_st | 3.147 | 0.9607 |  |
| zenpng_e14_st | 3.371 | 0.9597 |  |
| zenpng_e15_st | 4.619 | 0.9541 |  |
| zenpng_e16_st | 10.865 | 0.9468 |  |
| zenpng_e17_st | 13.851 | 0.9468 |  |
| zenpng_e18_st | 20.706 | 0.9466 |  |
| zenpng_e19_st | 38.116 | 0.9445 |  |

**tiny** (n=20)

| arm | time | size | dominated by |
|---|---|---|---|
| zune | 0.025 | 2.1312 |  |
| png_fast | 0.050 | 1.1223 |  |
| zenpng_e1_st | 0.095 | 1.0831 |  |
| zenpng_e1_mt | 0.151 | 1.0831 |  |
| zenpng_e2_st | 0.256 | 1.0569 |  |
| zenpng_e3_st | 0.340 | 1.0363 |  |
| zenpng_e4_st | 0.436 | 1.0242 |  |
| zenpng_e5_st | 0.454 | 1.0171 |  |
| png_balanced | 0.540 | 1.0183 |  |
| lodepng | 0.693 | 1.0059 |  |
| png_high | 1.000 | 1.0000 |  |
| zenpng_e6_st | 1.042 | 0.9992 |  |
| zenpng_e7_st | 1.109 | 0.9998 |  |
| zenpng_e8_st | 1.395 | 0.9997 |  |
| zenpng_e9_st | 1.569 | 0.9986 |  |
| zenpng_e10_st | 1.658 | 0.9970 |  |
| zenpng_e11_st | 1.716 | 0.9969 |  |
| zenpng_e12_st | 2.534 | 0.9963 |  |
| zenpng_e7_mt | 3.176 | 0.9998 |  |
| zenpng_e13_st | 6.830 | 0.9785 |  |
| zenpng_e14_st | 6.871 | 0.9785 |  |
| zenpng_e13_mt | 9.053 | 0.9785 |  |
| zenpng_e15_st | 9.958 | 0.9750 |  |
| zenpng_e16_st | 27.200 | 0.9697 |  |
| zenpng_e17_st | 35.055 | 0.9697 |  |
| zenpng_e18_st | 53.219 | 0.9680 |  |
| zenpng_e19_st | 90.420 | 0.9668 |  |

### enc by content: median time / png_high time, median size / png_high size

**document** (n=16)

| arm | time | size | dominated by |
|---|---|---|---|
| zune | 0.023 | 3.8239 |  |
| zenpng_e1_st | 0.030 | 1.1456 |  |
| png_fast | 0.032 | 1.1997 |  |
| zenpng_e1_mt | 0.037 | 1.1478 |  |
| zenpng_e2_st | 0.136 | 1.1045 |  |
| zenpng_e3_st | 0.193 | 1.0772 |  |
| zenpng_e4_st | 0.211 | 1.0651 |  |
| zenpng_e5_st | 0.232 | 1.0603 |  |
| png_balanced | 0.248 | 1.0562 |  |
| lodepng | 0.411 | 1.0491 |  |
| zenpng_e6_st | 0.503 | 1.0327 |  |
| zenpng_e7_st | 0.613 | 1.0266 |  |
| zenpng_e7_mt | 0.624 | 1.0269 |  |
| zenpng_e8_st | 0.848 | 1.0065 |  |
| zenpng_e9_st | 0.892 | 1.0049 |  |
| zenpng_e10_st | 0.950 | 1.0046 |  |
| zenpng_e11_st | 0.978 | 1.0046 |  |
| png_high | 1.000 | 1.0000 |  |
| zenpng_e12_st | 1.243 | 0.9994 |  |
| zenpng_e13_mt | 2.622 | 0.9689 |  |
| zenpng_e13_st | 3.310 | 0.9660 |  |
| zenpng_e14_st | 3.943 | 0.9625 |  |
| zenpng_e15_st | 7.018 | 0.9554 |  |
| zenpng_e16_st | 15.818 | 0.9432 |  |
| zenpng_e17_st | 25.095 | 0.9420 |  |
| zenpng_e18_st | 34.216 | 0.9410 |  |
| zenpng_e19_st | 71.973 | 0.9334 |  |

**lineart** (n=11)

| arm | time | size | dominated by |
|---|---|---|---|
| zune | 0.020 | 6.6755 |  |
| png_fast | 0.032 | 1.4790 |  |
| zenpng_e1_mt | 0.036 | 1.0957 |  |
| zenpng_e1_st | 0.046 | 1.0957 |  |
| zenpng_e2_st | 0.112 | 1.0677 |  |
| zenpng_e3_st | 0.161 | 1.0677 |  |
| zenpng_e4_st | 0.174 | 1.0578 |  |
| zenpng_e5_st | 0.187 | 1.0567 |  |
| png_balanced | 0.197 | 1.0732 |  |
| lodepng | 0.326 | 1.0436 |  |
| zenpng_e6_st | 0.414 | 1.0418 |  |
| zenpng_e7_mt | 0.462 | 1.0362 |  |
| zenpng_e7_st | 0.499 | 1.0329 |  |
| zenpng_e8_st | 0.688 | 1.0221 |  |
| zenpng_e9_st | 0.761 | 1.0172 |  |
| zenpng_e10_st | 0.879 | 1.0112 |  |
| zenpng_e11_st | 0.934 | 1.0117 |  |
| png_high | 1.000 | 1.0000 |  |
| zenpng_e12_st | 1.166 | 0.9992 |  |
| zenpng_e13_mt | 2.227 | 0.9452 |  |
| zenpng_e13_st | 3.147 | 0.9452 |  |
| zenpng_e14_st | 3.479 | 0.9439 |  |
| zenpng_e15_st | 5.372 | 0.9417 |  |
| zenpng_e16_st | 12.148 | 0.9309 |  |
| zenpng_e17_st | 17.351 | 0.9309 |  |
| zenpng_e18_st | 24.399 | 0.9309 |  |
| zenpng_e19_st | 51.342 | 0.9309 |  |

**mixed** (n=9)

| arm | time | size | dominated by |
|---|---|---|---|
| zune | 0.027 | 1.5171 |  |
| zenpng_e1_st | 0.034 | 1.0652 |  |
| zenpng_e1_mt | 0.041 | 1.0652 |  |
| png_fast | 0.060 | 1.1107 |  |
| zenpng_e2_st | 0.158 | 1.0569 |  |
| zenpng_e3_st | 0.205 | 1.0338 |  |
| zenpng_e4_st | 0.253 | 1.0219 |  |
| zenpng_e5_st | 0.267 | 1.0144 |  |
| png_balanced | 0.598 | 1.0084 |  |
| lodepng | 0.694 | 1.0020 |  |
| zenpng_e6_st | 0.863 | 0.9978 |  |
| zenpng_e7_st | 0.871 | 0.9991 |  |
| png_high | 1.000 | 1.0000 |  |
| zenpng_e8_st | 1.008 | 0.9991 |  |
| zenpng_e7_mt | 1.059 | 0.9991 |  |
| zenpng_e9_st | 1.081 | 0.9986 |  |
| zenpng_e10_st | 1.180 | 0.9986 |  |
| zenpng_e11_st | 1.206 | 0.9986 |  |
| zenpng_e12_st | 2.488 | 0.9978 |  |
| zenpng_e13_mt | 3.373 | 0.9771 |  |
| zenpng_e14_st | 4.134 | 0.9738 |  |
| zenpng_e13_st | 4.144 | 0.9739 |  |
| zenpng_e15_st | 4.646 | 0.9737 |  |
| zenpng_e16_st | 12.222 | 0.9688 |  |
| zenpng_e17_st | 15.214 | 0.9688 |  |
| zenpng_e18_st | 23.374 | 0.9679 |  |
| zenpng_e19_st | 40.048 | 0.9679 |  |

**photo** (n=26)

| arm | time | size | dominated by |
|---|---|---|---|
| zune | 0.016 | 2.3168 |  |
| zenpng_e1_st | 0.023 | 1.1203 |  |
| zenpng_e1_mt | 0.027 | 1.1170 |  |
| png_fast | 0.031 | 1.1197 |  |
| zenpng_e2_st | 0.087 | 1.0846 |  |
| zenpng_e3_st | 0.147 | 1.0587 |  |
| zenpng_e4_st | 0.227 | 1.0414 |  |
| zenpng_e5_st | 0.253 | 1.0302 |  |
| png_balanced | 0.276 | 1.0122 |  |
| lodepng | 0.475 | 0.9980 |  |
| zenpng_e7_mt | 0.491 | 1.0073 | lodepng |
| zenpng_e6_st | 0.583 | 1.0163 | lodepng, png_balanced |
| zenpng_e7_st | 0.693 | 1.0066 | lodepng |
| zenpng_e8_st | 0.885 | 1.0066 | lodepng |
| zenpng_e9_st | 0.955 | 1.0032 | lodepng |
| png_high | 1.000 | 1.0000 |  |
| zenpng_e10_st | 1.055 | 0.9999 | lodepng |
| zenpng_e11_st | 1.087 | 0.9995 | lodepng |
| zenpng_e12_st | 1.625 | 0.9995 | lodepng |
| zenpng_e13_mt | 2.513 | 0.9711 |  |
| zenpng_e13_st | 3.516 | 0.9700 |  |
| zenpng_e14_st | 3.641 | 0.9700 |  |
| zenpng_e15_st | 4.556 | 0.9661 |  |
| zenpng_e16_st | 10.121 | 0.9572 |  |
| zenpng_e17_st | 13.599 | 0.9572 |  |
| zenpng_e18_st | 20.473 | 0.9566 |  |
| zenpng_e19_st | 37.205 | 0.9543 |  |

**screen** (n=8)

| arm | time | size | dominated by |
|---|---|---|---|
| zune | 0.025 | 6.3333 |  |
| zenpng_e1_st | 0.034 | 1.3201 |  |
| png_fast | 0.035 | 1.4393 |  |
| zenpng_e1_mt | 0.048 | 1.3152 |  |
| zenpng_e2_st | 0.146 | 1.2756 |  |
| zenpng_e3_st | 0.208 | 1.1744 |  |
| png_balanced | 0.212 | 1.0955 |  |
| zenpng_e4_st | 0.230 | 1.1573 | png_balanced |
| zenpng_e5_st | 0.244 | 1.1362 | png_balanced |
| lodepng | 0.374 | 1.0813 |  |
| zenpng_e6_st | 0.510 | 1.0403 |  |
| zenpng_e7_st | 0.585 | 1.0313 |  |
| zenpng_e7_mt | 0.618 | 1.0334 |  |
| zenpng_e8_st | 0.827 | 1.0241 |  |
| zenpng_e9_st | 0.891 | 1.0232 |  |
| zenpng_e10_st | 0.973 | 1.0157 |  |
| png_high | 1.000 | 1.0000 |  |
| zenpng_e11_st | 1.025 | 1.0136 | png_high |
| zenpng_e12_st | 1.272 | 1.0136 | png_high |
| zenpng_e13_mt | 3.378 | 0.9775 |  |
| zenpng_e13_st | 4.465 | 0.9665 |  |
| zenpng_e14_st | 5.015 | 0.9632 |  |
| zenpng_e15_st | 8.577 | 0.9500 |  |
| zenpng_e16_st | 20.168 | 0.9362 |  |
| zenpng_e17_st | 31.629 | 0.9362 |  |
| zenpng_e18_st | 44.706 | 0.9334 |  |
| zenpng_e19_st | 83.552 | 0.9284 |  |



---

# Rerun: e13 and e15-e19 after reshaping e17-e19 (shipped configs)

The first run above measured e17-e19 as None+Paeth+MinSum + BF / the 9-strategy
screen; they now refine at png(26,28[,30]) with BF (3,1)[,(5,1)] (e17 Thorough,
e19 High). e1-e16 are unchanged, so the tables above stand for them; this run
(same host, inputs and command with `ZENPNG_PARETO_EFFORTS=13,15,16,17,18,19`,
no MT arms) is the record for e17-e19. Run 2026-10-06 ~19:30 UTC.

## Per content class (geomean vs png High)

| class (n) | rung | size | time |
|---|---|---|---|
| photo (26) | Balanced e13 | 0.9690 | 3.69 |
| photo (26) | e15 | 0.9646 | 5.13 |
| photo (26) | e16 | 0.9572 | 11.72 |
| photo (26) | Thorough e17 | 0.9570 | 15.85 |
| photo (26) | e18 | 0.9570 | 20.68 |
| photo (26) | High e19 | 0.9562 | 27.91 |
| screen (8) | Balanced e13 | 0.9593 | 4.02 |
| screen (8) | e15 | 0.9447 | 7.49 |
| screen (8) | e16 | 0.9374 | 16.42 |
| screen (8) | Thorough e17 | 0.9373 | 25.35 |
| screen (8) | e18 | 0.9364 | 35.58 |
| screen (8) | High e19 | 0.9356 | 44.98 |
| lineart (11) | Balanced e13 | 0.8401 | 3.33 |
| lineart (11) | e15 | 0.8273 | 5.58 |
| lineart (11) | e16 | 0.8244 | 12.78 |
| lineart (11) | Thorough e17 | 0.8214 | 18.91 |
| lineart (11) | e18 | 0.8205 | 26.41 |
| lineart (11) | High e19 | 0.8204 | 33.95 |
| document (16) | Balanced e13 | 0.9450 | 3.69 |
| document (16) | e15 | 0.9344 | 6.03 |
| document (16) | e16 | 0.9280 | 13.75 |
| document (16) | Thorough e17 | 0.9261 | 19.65 |
| document (16) | e18 | 0.9248 | 26.70 |
| document (16) | High e19 | 0.9237 | 35.04 |
| mixed (9) | Balanced e13 | 0.9727 | 4.30 |
| mixed (9) | e15 | 0.9720 | 5.22 |
| mixed (9) | e16 | 0.9700 | 13.95 |
| mixed (9) | Thorough e17 | 0.9700 | 17.43 |
| mixed (9) | e18 | 0.9700 | 21.47 |
| mixed (9) | High e19 | 0.9696 | 31.50 |

## enc

| arm | n | ms = a + b·MP (a ms, b ms/MP; relative-error fit) | bytes = a + b·MP (enc) |
|---|---|---|---|
| lodepng | 70 | 0.0517 + 23.996·MP | 2123 + 353715·MP |
| png_balanced | 70 | 0.0706 + 11.727·MP | 2380 + 458203·MP |
| png_fast | 70 | 0.0054 + 2.384·MP | 2555 + 593990·MP |
| png_high | 70 | -0.0132 + 65.647·MP | 2341 + 381284·MP |
| zenpng_e13_st | 70 | 0.3731 + 235.735·MP | 2411 + 278719·MP |
| zenpng_e15_st | 70 | 0.3232 + 410.747·MP | 2405 + 273719·MP |
| zenpng_e16_st | 70 | 1.8182 + 933.428·MP | 2403 + 273651·MP |
| zenpng_e17_st | 70 | 1.7363 + 1373.535·MP | 2408 + 271542·MP |
| zenpng_e18_st | 70 | 1.7415 + 1826.422·MP | 2410 + 270397·MP |
| zenpng_e19_st | 70 | 3.0747 + 2476.687·MP | 2408 + 270387·MP |
| zune | 70 | -0.0006 + 1.991·MP | -241 + 3132974·MP |

### enc by size_class: median time / png_high time, median size / png_high size

**medium** (n=25)

| arm | time | size | dominated by |
|---|---|---|---|
| zune | 0.019 | 3.7228 |  |
| png_fast | 0.024 | 1.2117 |  |
| png_balanced | 0.156 | 1.0309 |  |
| lodepng | 0.303 | 1.0142 |  |
| png_high | 1.000 | 1.0000 |  |
| zenpng_e13_st | 2.951 | 0.9622 |  |
| zenpng_e15_st | 4.150 | 0.9514 |  |
| zenpng_e16_st | 8.234 | 0.9415 |  |
| zenpng_e17_st | 12.519 | 0.9415 |  |
| zenpng_e18_st | 15.982 | 0.9415 |  |
| zenpng_e19_st | 22.197 | 0.9410 |  |

**small** (n=25)

| arm | time | size | dominated by |
|---|---|---|---|
| zune | 0.017 | 3.0251 |  |
| png_fast | 0.033 | 1.1826 |  |
| png_balanced | 0.230 | 1.0463 |  |
| lodepng | 0.372 | 1.0234 |  |
| png_high | 1.000 | 1.0000 |  |
| zenpng_e13_st | 3.128 | 0.9607 |  |
| zenpng_e15_st | 4.622 | 0.9541 |  |
| zenpng_e16_st | 10.909 | 0.9468 |  |
| zenpng_e17_st | 13.920 | 0.9468 |  |
| zenpng_e18_st | 18.888 | 0.9468 |  |
| zenpng_e19_st | 25.486 | 0.9466 |  |

**tiny** (n=20)

| arm | time | size | dominated by |
|---|---|---|---|
| zune | 0.026 | 2.1312 |  |
| png_fast | 0.067 | 1.1223 |  |
| png_balanced | 0.561 | 1.0183 |  |
| lodepng | 0.712 | 1.0059 |  |
| png_high | 1.000 | 1.0000 |  |
| zenpng_e13_st | 6.327 | 0.9785 |  |
| zenpng_e15_st | 9.259 | 0.9750 |  |
| zenpng_e16_st | 25.274 | 0.9697 |  |
| zenpng_e17_st | 32.536 | 0.9697 |  |
| zenpng_e18_st | 39.774 | 0.9697 |  |
| zenpng_e19_st | 58.686 | 0.9674 |  |

### enc by content: median time / png_high time, median size / png_high size

**document** (n=16)

| arm | time | size | dominated by |
|---|---|---|---|
| zune | 0.023 | 3.8239 |  |
| png_fast | 0.032 | 1.1997 |  |
| png_balanced | 0.247 | 1.0562 |  |
| lodepng | 0.409 | 1.0491 |  |
| png_high | 1.000 | 1.0000 |  |
| zenpng_e13_st | 3.317 | 0.9660 |  |
| zenpng_e15_st | 6.994 | 0.9554 |  |
| zenpng_e16_st | 15.751 | 0.9432 |  |
| zenpng_e17_st | 25.040 | 0.9420 |  |
| zenpng_e18_st | 35.353 | 0.9420 |  |
| zenpng_e19_st | 45.338 | 0.9408 |  |

**lineart** (n=11)

| arm | time | size | dominated by |
|---|---|---|---|
| zune | 0.021 | 6.6755 |  |
| png_fast | 0.034 | 1.4790 |  |
| png_balanced | 0.201 | 1.0732 |  |
| lodepng | 0.319 | 1.0436 |  |
| png_high | 1.000 | 1.0000 |  |
| zenpng_e13_st | 3.107 | 0.9452 |  |
| zenpng_e15_st | 5.395 | 0.9417 |  |
| zenpng_e16_st | 12.048 | 0.9309 |  |
| zenpng_e17_st | 17.394 | 0.9309 |  |
| zenpng_e18_st | 24.894 | 0.9309 |  |
| zenpng_e19_st | 31.180 | 0.9309 |  |

**mixed** (n=9)

| arm | time | size | dominated by |
|---|---|---|---|
| zune | 0.027 | 1.5171 |  |
| png_fast | 0.063 | 1.1107 |  |
| png_balanced | 0.608 | 1.0084 |  |
| lodepng | 0.711 | 1.0020 |  |
| png_high | 1.000 | 1.0000 |  |
| zenpng_e13_st | 4.087 | 0.9739 |  |
| zenpng_e15_st | 4.630 | 0.9737 |  |
| zenpng_e16_st | 12.080 | 0.9688 |  |
| zenpng_e17_st | 15.128 | 0.9688 |  |
| zenpng_e18_st | 18.888 | 0.9688 |  |
| zenpng_e19_st | 27.358 | 0.9679 |  |

**photo** (n=26)

| arm | time | size | dominated by |
|---|---|---|---|
| zune | 0.016 | 2.3168 |  |
| png_fast | 0.031 | 1.1197 |  |
| png_balanced | 0.276 | 1.0122 |  |
| lodepng | 0.472 | 0.9980 |  |
| png_high | 1.000 | 1.0000 |  |
| zenpng_e13_st | 3.518 | 0.9700 |  |
| zenpng_e15_st | 4.546 | 0.9661 |  |
| zenpng_e16_st | 10.148 | 0.9572 |  |
| zenpng_e17_st | 13.652 | 0.9572 |  |
| zenpng_e18_st | 17.953 | 0.9572 |  |
| zenpng_e19_st | 24.398 | 0.9566 |  |

**screen** (n=8)

| arm | time | size | dominated by |
|---|---|---|---|
| zune | 0.027 | 6.3333 |  |
| png_fast | 0.037 | 1.4393 |  |
| png_balanced | 0.212 | 1.0955 |  |
| lodepng | 0.376 | 1.0813 |  |
| png_high | 1.000 | 1.0000 |  |
| zenpng_e13_st | 4.275 | 0.9665 |  |
| zenpng_e15_st | 8.113 | 0.9500 |  |
| zenpng_e16_st | 20.106 | 0.9362 |  |
| zenpng_e17_st | 29.866 | 0.9362 |  |
| zenpng_e18_st | 40.656 | 0.9362 |  |
| zenpng_e19_st | 53.042 | 0.9334 |  |

