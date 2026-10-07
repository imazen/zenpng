# benches/pareto.rs --group=enc, x86 i265 (Core Ultra 7 265K) P-cores (taskset -c 0-7), nice 19, zenbench lock. 2026-10-07.
# zenpng 0f75a1b0bf2ca1bfed8bd92c11e534e4294e42b0 (ladder as landed 2026-10-06). zenflate png-mode 66ead6f739cb3be4ba27ad41230c9da5938ec719 (clean).
# 70 RGB8/RGBA8 inputs at 64/256/1024 px (~/tmp/pareto_in_le1024, same as pareto_ladder_x86_2026-10-06.md). ZENPNG_PARETO_EFFORTS=1..19, MT_EFFORTS=1,7,13, NO_IDOT=1.
# Result: within noise of the 2026-10-06 run (zenflate's changes since 8b8cf0f are decode-side); e.g. medium e7 0.491x/1.0194, e13 2.982x/0.9622, e19 22.48x/0.9410 of png High time/size.
# Not included: 5e32a2e (RGBA8 downcast analysis early exit) and 0925d00 (top-k screening buffers), which postdate the build.


(Full tables omitted to keep this record under 30 KB; the final run below supersedes it. Raw: i265 ~/tmp/pareto_0707_enc*.)

---

# Rerun 2026-10-07 (superseded by the FINAL record below): zenpng 0908614 (e11-e14 re-spread on the png(19..23) ramp, RGBA8 analysis early exit, top-k screening buffers), zenflate main 1817ce8 (clean). Same host, inputs and command.
# Medium (1024 px) medians vs png High (time / size): e1 0.014 / 1.1516, e7 0.486 / 1.0194, e11 1.076 / 1.0019, e12 2.839 / 0.9672, e13 (Balanced) 3.134 / 0.9587, e15 4.273 / 0.9529, e19 (High) 23.35 / 0.9412.
# e20-e30 (rebuilt in 128ae10, after this build) are not in this run; see CLAUDE.md 'Upper ladder'.


(Full tables omitted: superseded by the final run below, which has identical output sizes. Raw: i265 ~/tmp/pareto_0707d_enc*.)

---

# FINAL record 2026-10-07: zenpng 451c491 (ladder as shipped, incl. e20-e30 rebuild 128ae10), zenflate main 3d639ec (= 52af148a matchfinder/DP speedups + a no_std import fix). Same host (i265 P-cores 0-7, zenbench lock), inputs and command as above.
# vs the 1817ce8 run above (1024 px, per-image median ratios): sizes identical at every effort; time e1 0.953, e7 0.991, e11 1.002, e12 0.928, e13 0.922, e14 0.923, e15 0.893, e16 0.900, e19 0.871.
# Medium (1024 px) medians vs png High (time / size): e1 0.015 / 1.1516, e7 0.480 / 1.0194, e11 1.089 / 1.0019, e12 2.660 / 0.9672, e13 (Balanced) 2.812 / 0.9587, e15 3.697 / 0.9526, e16 7.586 / 0.9418, e19 (High) 19.94 / 0.9412.
# Per-image monotonicity on this zenflate (91 images, roundtrip_sweep): unchanged from CLAUDE.md (e7->e8 +1.95%, e13->e14 +0.381%, all else <= 0.13%).

## enc

| arm | n | ms = a + b·MP (a ms, b ms/MP; relative-error fit) | bytes = a + b·MP (enc) |
|---|---|---|---|
| lodepng | 70 | 0.0407 + 23.656·MP | 2122 + 353845·MP |
| png_balanced | 70 | 0.0577 + 11.648·MP | 2380 + 458235·MP |
| png_fast | 70 | 0.0020 + 2.365·MP | 2555 + 593919·MP |
| png_high | 70 | -0.0277 + 64.946·MP | 2341 + 381355·MP |
| zenpng_e10_st | 70 | 0.0878 + 52.639·MP | 2391 + 297477·MP |
| zenpng_e11_st | 70 | 0.1504 + 73.934·MP | 2391 + 296625·MP |
| zenpng_e12_st | 70 | 0.4913 + 178.406·MP | 2403 + 284800·MP |
| zenpng_e13_mt | 70 | 1.2674 + 115.618·MP | 2407 + 279054·MP |
| zenpng_e13_st | 70 | 0.2600 + 274.660·MP | 2410 + 278356·MP |
| zenpng_e14_st | 70 | 0.0878 + 338.062·MP | 2416 + 276174·MP |
| zenpng_e15_st | 70 | 0.3331 + 395.823·MP | 2401 + 274800·MP |
| zenpng_e16_st | 70 | 1.8120 + 892.959·MP | 2400 + 274768·MP |
| zenpng_e17_st | 70 | 1.7552 + 1306.626·MP | 2403 + 272991·MP |
| zenpng_e18_st | 70 | 1.8039 + 1716.991·MP | 2405 + 271794·MP |
| zenpng_e19_st | 70 | 3.0963 + 2339.630·MP | 2404 + 271743·MP |
| zenpng_e1_mt | 70 | 0.0220 + 2.266·MP | 2437 + 381398·MP |
| zenpng_e1_st | 70 | 0.0120 + 1.896·MP | 2442 + 379512·MP |
| zenpng_e2_st | 70 | 0.0187 + 9.098·MP | 2377 + 366525·MP |
| zenpng_e3_st | 70 | 0.0199 + 12.588·MP | 2361 + 354386·MP |
| zenpng_e4_st | 70 | 0.0268 + 13.402·MP | 2348 + 352029·MP |
| zenpng_e5_st | 70 | 0.0275 + 13.986·MP | 2341 + 349948·MP |
| zenpng_e6_st | 70 | 0.0581 + 30.183·MP | 2351 + 330106·MP |
| zenpng_e7_mt | 70 | 0.5153 + 14.649·MP | 2345 + 328067·MP |
| zenpng_e7_st | 70 | 0.0589 + 33.948·MP | 2344 + 328155·MP |
| zenpng_e8_st | 70 | 0.0693 + 45.114·MP | 2396 + 299172·MP |
| zenpng_e9_st | 70 | 0.0937 + 47.978·MP | 2392 + 298613·MP |
| zune | 70 | -0.0012 + 1.976·MP | -239 + 3132444·MP |

### enc by size_class: median time / png_high time, median size / png_high size

**medium** (n=25)

| arm | time | size | dominated by |
|---|---|---|---|
| zenpng_e1_st | 0.015 | 1.1516 |  |
| zune | 0.015 | 3.7228 |  |
| zenpng_e1_mt | 0.016 | 1.1445 |  |
| png_fast | 0.023 | 1.2117 |  |
| zenpng_e2_st | 0.071 | 1.1088 |  |
| zenpng_e3_st | 0.129 | 1.0923 |  |
| png_balanced | 0.157 | 1.0309 |  |
| zenpng_e4_st | 0.158 | 1.0753 | png_balanced |
| zenpng_e5_st | 0.166 | 1.0542 | png_balanced |
| zenpng_e7_mt | 0.176 | 1.0214 |  |
| lodepng | 0.306 | 1.0142 |  |
| zenpng_e6_st | 0.425 | 1.0357 | lodepng, png_balanced |
| zenpng_e7_st | 0.480 | 1.0194 | lodepng |
| zenpng_e8_st | 0.619 | 1.0081 |  |
| zenpng_e9_st | 0.662 | 1.0065 |  |
| zenpng_e10_st | 0.765 | 1.0039 |  |
| zenpng_e13_mt | 0.906 | 0.9634 |  |
| png_high | 1.000 | 1.0000 |  |
| zenpng_e11_st | 1.089 | 1.0019 | png_high |
| zenpng_e12_st | 2.660 | 0.9672 |  |
| zenpng_e13_st | 2.812 | 0.9587 |  |
| zenpng_e14_st | 2.816 | 0.9569 |  |
| zenpng_e15_st | 3.697 | 0.9526 |  |
| zenpng_e16_st | 7.586 | 0.9418 |  |
| zenpng_e17_st | 11.355 | 0.9418 |  |
| zenpng_e18_st | 14.493 | 0.9418 |  |
| zenpng_e19_st | 19.941 | 0.9412 |  |

**small** (n=25)

| arm | time | size | dominated by |
|---|---|---|---|
| zune | 0.018 | 3.0251 |  |
| zenpng_e1_st | 0.025 | 1.1393 |  |
| zenpng_e1_mt | 0.033 | 1.1393 |  |
| png_fast | 0.033 | 1.1826 |  |
| zenpng_e2_st | 0.101 | 1.1020 |  |
| zenpng_e3_st | 0.159 | 1.0769 |  |
| zenpng_e4_st | 0.179 | 1.0577 |  |
| zenpng_e5_st | 0.207 | 1.0557 |  |
| png_balanced | 0.229 | 1.0463 |  |
| lodepng | 0.373 | 1.0234 |  |
| zenpng_e6_st | 0.455 | 1.0277 | lodepng |
| zenpng_e7_st | 0.542 | 1.0214 |  |
| zenpng_e7_mt | 0.648 | 1.0214 |  |
| zenpng_e8_st | 0.744 | 1.0196 |  |
| zenpng_e9_st | 0.784 | 1.0189 |  |
| zenpng_e10_st | 0.879 | 1.0162 |  |
| png_high | 1.000 | 1.0000 |  |
| zenpng_e11_st | 1.235 | 1.0066 | png_high |
| zenpng_e12_st | 2.753 | 0.9723 |  |
| zenpng_e13_mt | 2.879 | 0.9669 |  |
| zenpng_e13_st | 3.243 | 0.9669 |  |
| zenpng_e14_st | 3.451 | 0.9655 |  |
| zenpng_e15_st | 4.122 | 0.9606 |  |
| zenpng_e16_st | 9.874 | 0.9486 |  |
| zenpng_e17_st | 12.996 | 0.9485 |  |
| zenpng_e18_st | 16.491 | 0.9485 |  |
| zenpng_e19_st | 22.359 | 0.9484 |  |

**tiny** (n=20)

| arm | time | size | dominated by |
|---|---|---|---|
| zune | 0.025 | 2.1312 |  |
| png_fast | 0.051 | 1.1223 |  |
| zenpng_e1_st | 0.095 | 1.0831 |  |
| zenpng_e1_mt | 0.150 | 1.0831 |  |
| zenpng_e2_st | 0.254 | 1.0569 |  |
| zenpng_e3_st | 0.339 | 1.0363 |  |
| zenpng_e4_st | 0.422 | 1.0242 |  |
| zenpng_e5_st | 0.443 | 1.0171 |  |
| png_balanced | 0.536 | 1.0183 |  |
| lodepng | 0.694 | 1.0059 |  |
| png_high | 1.000 | 1.0000 |  |
| zenpng_e6_st | 1.027 | 0.9995 |  |
| zenpng_e7_st | 1.095 | 0.9998 |  |
| zenpng_e8_st | 1.388 | 0.9997 |  |
| zenpng_e9_st | 1.562 | 0.9986 |  |
| zenpng_e10_st | 1.630 | 0.9970 |  |
| zenpng_e11_st | 2.529 | 0.9963 |  |
| zenpng_e7_mt | 3.177 | 0.9998 |  |
| zenpng_e12_st | 6.470 | 0.9787 |  |
| zenpng_e13_st | 6.867 | 0.9785 |  |
| zenpng_e14_st | 6.955 | 0.9785 |  |
| zenpng_e13_mt | 9.193 | 0.9785 |  |
| zenpng_e15_st | 9.760 | 0.9750 |  |
| zenpng_e16_st | 26.666 | 0.9697 |  |
| zenpng_e17_st | 34.115 | 0.9697 |  |
| zenpng_e18_st | 42.561 | 0.9697 |  |
| zenpng_e19_st | 60.605 | 0.9674 |  |

### enc by content: median time / png_high time, median size / png_high size

**document** (n=16)

| arm | time | size | dominated by |
|---|---|---|---|
| zune | 0.023 | 3.8239 |  |
| zenpng_e1_st | 0.027 | 1.1456 |  |
| png_fast | 0.032 | 1.1997 |  |
| zenpng_e1_mt | 0.039 | 1.1478 |  |
| zenpng_e2_st | 0.134 | 1.1045 |  |
| zenpng_e3_st | 0.189 | 1.0772 |  |
| zenpng_e4_st | 0.205 | 1.0651 |  |
| zenpng_e5_st | 0.218 | 1.0603 |  |
| png_balanced | 0.248 | 1.0562 |  |
| lodepng | 0.413 | 1.0491 |  |
| zenpng_e6_st | 0.491 | 1.0327 |  |
| zenpng_e7_st | 0.609 | 1.0266 |  |
| zenpng_e7_mt | 0.612 | 1.0269 |  |
| zenpng_e8_st | 0.844 | 1.0065 |  |
| zenpng_e9_st | 0.889 | 1.0049 |  |
| zenpng_e10_st | 0.948 | 1.0046 |  |
| png_high | 1.000 | 1.0000 |  |
| zenpng_e11_st | 1.262 | 0.9994 |  |
| zenpng_e13_mt | 2.508 | 0.9628 |  |
| zenpng_e12_st | 2.942 | 0.9672 |  |
| zenpng_e13_st | 4.167 | 0.9628 |  |
| zenpng_e14_st | 5.341 | 0.9617 |  |
| zenpng_e15_st | 7.628 | 0.9566 |  |
| zenpng_e16_st | 16.892 | 0.9459 |  |
| zenpng_e17_st | 26.404 | 0.9459 |  |
| zenpng_e18_st | 35.476 | 0.9459 |  |
| zenpng_e19_st | 47.091 | 0.9428 |  |

**lineart** (n=11)

| arm | time | size | dominated by |
|---|---|---|---|
| zune | 0.020 | 6.6755 |  |
| png_fast | 0.031 | 1.4790 |  |
| zenpng_e1_mt | 0.037 | 1.0957 |  |
| zenpng_e1_st | 0.040 | 1.0957 |  |
| zenpng_e2_st | 0.110 | 1.0677 |  |
| zenpng_e3_st | 0.159 | 1.0677 |  |
| zenpng_e4_st | 0.171 | 1.0578 |  |
| zenpng_e5_st | 0.181 | 1.0577 |  |
| png_balanced | 0.197 | 1.0732 |  |
| lodepng | 0.322 | 1.0436 |  |
| zenpng_e6_st | 0.414 | 1.0418 |  |
| zenpng_e7_mt | 0.456 | 1.0362 |  |
| zenpng_e7_st | 0.495 | 1.0329 |  |
| zenpng_e8_st | 0.677 | 1.0221 |  |
| zenpng_e9_st | 0.767 | 1.0172 |  |
| zenpng_e10_st | 0.879 | 1.0112 |  |
| png_high | 1.000 | 1.0000 |  |
| zenpng_e11_st | 1.161 | 0.9992 |  |
| zenpng_e13_mt | 2.244 | 0.9474 |  |
| zenpng_e12_st | 2.753 | 0.9907 |  |
| zenpng_e13_st | 3.305 | 0.9474 |  |
| zenpng_e14_st | 3.451 | 0.9445 |  |
| zenpng_e15_st | 5.166 | 0.9423 |  |
| zenpng_e16_st | 12.325 | 0.9376 |  |
| zenpng_e17_st | 16.880 | 0.9330 |  |
| zenpng_e18_st | 24.311 | 0.9316 |  |
| zenpng_e19_st | 30.380 | 0.9316 |  |

**mixed** (n=9)

| arm | time | size | dominated by |
|---|---|---|---|
| zune | 0.027 | 1.5171 |  |
| zenpng_e1_st | 0.031 | 1.0652 |  |
| zenpng_e1_mt | 0.040 | 1.0652 |  |
| png_fast | 0.060 | 1.1107 |  |
| zenpng_e2_st | 0.153 | 1.0569 |  |
| zenpng_e3_st | 0.206 | 1.0338 |  |
| zenpng_e4_st | 0.234 | 1.0219 |  |
| zenpng_e5_st | 0.253 | 1.0144 |  |
| png_balanced | 0.582 | 1.0084 |  |
| lodepng | 0.688 | 1.0020 |  |
| zenpng_e6_st | 0.850 | 0.9978 |  |
| zenpng_e7_st | 0.901 | 0.9991 |  |
| png_high | 1.000 | 1.0000 |  |
| zenpng_e8_st | 1.009 | 0.9991 |  |
| zenpng_e7_mt | 1.049 | 0.9991 |  |
| zenpng_e9_st | 1.056 | 0.9986 |  |
| zenpng_e10_st | 1.152 | 0.9986 |  |
| zenpng_e11_st | 2.567 | 0.9978 |  |
| zenpng_e13_mt | 3.254 | 0.9774 |  |
| zenpng_e14_st | 4.113 | 0.9771 |  |
| zenpng_e12_st | 4.113 | 0.9779 |  |
| zenpng_e13_st | 4.114 | 0.9774 |  |
| zenpng_e15_st | 4.570 | 0.9743 |  |
| zenpng_e16_st | 11.399 | 0.9688 |  |
| zenpng_e17_st | 14.071 | 0.9688 |  |
| zenpng_e18_st | 17.206 | 0.9688 |  |
| zenpng_e19_st | 26.017 | 0.9679 |  |

**photo** (n=26)

| arm | time | size | dominated by |
|---|---|---|---|
| zune | 0.016 | 2.3168 |  |
| zenpng_e1_st | 0.021 | 1.1203 |  |
| zenpng_e1_mt | 0.026 | 1.1170 |  |
| png_fast | 0.031 | 1.1197 |  |
| zenpng_e2_st | 0.086 | 1.0846 |  |
| zenpng_e3_st | 0.144 | 1.0587 |  |
| zenpng_e4_st | 0.219 | 1.0414 |  |
| zenpng_e5_st | 0.238 | 1.0302 |  |
| png_balanced | 0.275 | 1.0122 |  |
| lodepng | 0.477 | 0.9980 |  |
| zenpng_e7_mt | 0.481 | 1.0073 | lodepng |
| zenpng_e6_st | 0.571 | 1.0163 | lodepng, png_balanced |
| zenpng_e7_st | 0.685 | 1.0065 | lodepng |
| zenpng_e8_st | 0.872 | 1.0065 | lodepng |
| zenpng_e9_st | 0.947 | 1.0032 | lodepng |
| png_high | 1.000 | 1.0000 |  |
| zenpng_e10_st | 1.039 | 0.9999 | lodepng |
| zenpng_e11_st | 1.626 | 0.9995 | lodepng |
| zenpng_e13_mt | 2.352 | 0.9721 |  |
| zenpng_e12_st | 3.145 | 0.9729 |  |
| zenpng_e13_st | 3.414 | 0.9697 |  |
| zenpng_e14_st | 3.521 | 0.9697 |  |
| zenpng_e15_st | 4.079 | 0.9665 |  |
| zenpng_e16_st | 9.217 | 0.9603 |  |
| zenpng_e17_st | 12.575 | 0.9590 |  |
| zenpng_e18_st | 15.605 | 0.9590 |  |
| zenpng_e19_st | 21.971 | 0.9590 |  |

**screen** (n=8)

| arm | time | size | dominated by |
|---|---|---|---|
| zune | 0.025 | 6.3333 |  |
| zenpng_e1_st | 0.034 | 1.3201 |  |
| png_fast | 0.035 | 1.4393 |  |
| zenpng_e1_mt | 0.047 | 1.3152 |  |
| zenpng_e2_st | 0.145 | 1.2756 |  |
| zenpng_e3_st | 0.207 | 1.1744 |  |
| png_balanced | 0.212 | 1.0955 |  |
| zenpng_e4_st | 0.226 | 1.1577 | png_balanced |
| zenpng_e5_st | 0.237 | 1.1364 | png_balanced |
| lodepng | 0.375 | 1.0813 |  |
| zenpng_e6_st | 0.508 | 1.0403 |  |
| zenpng_e7_st | 0.584 | 1.0313 |  |
| zenpng_e7_mt | 0.626 | 1.0334 |  |
| zenpng_e8_st | 0.824 | 1.0241 |  |
| zenpng_e9_st | 0.886 | 1.0232 |  |
| zenpng_e10_st | 0.972 | 1.0157 |  |
| png_high | 1.000 | 1.0000 |  |
| zenpng_e11_st | 1.284 | 1.0136 | png_high |
| zenpng_e12_st | 3.082 | 0.9894 |  |
| zenpng_e13_mt | 4.026 | 0.9707 |  |
| zenpng_e13_st | 4.824 | 0.9652 |  |
| zenpng_e14_st | 5.601 | 0.9592 |  |
| zenpng_e15_st | 8.144 | 0.9518 |  |
| zenpng_e16_st | 18.445 | 0.9403 |  |
| zenpng_e17_st | 31.089 | 0.9403 |  |
| zenpng_e18_st | 42.144 | 0.9403 |  |
| zenpng_e19_st | 55.137 | 0.9348 |  |

