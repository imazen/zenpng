# benches/pareto.rs --group=enc, x86 i265 (Core Ultra 7 265K) P-cores (taskset -c 0-7), nice 19, zenbench lock. 2026-10-07.
# zenpng 0f75a1b0bf2ca1bfed8bd92c11e534e4294e42b0 (ladder as landed 2026-10-06). zenflate png-mode 66ead6f739cb3be4ba27ad41230c9da5938ec719 (clean).
# 70 RGB8/RGBA8 inputs at 64/256/1024 px (~/tmp/pareto_in_le1024, same as pareto_ladder_x86_2026-10-06.md). ZENPNG_PARETO_EFFORTS=1..19, MT_EFFORTS=1,7,13, NO_IDOT=1.
# Result: within noise of the 2026-10-06 run (zenflate's changes since 8b8cf0f are decode-side); e.g. medium e7 0.491x/1.0194, e13 2.982x/0.9622, e19 22.48x/0.9410 of png High time/size.
# Not included: 5e32a2e (RGBA8 downcast analysis early exit) and 0925d00 (top-k screening buffers), which postdate the build.

## enc

| arm | n | ms = a + b·MP (a ms, b ms/MP; relative-error fit) | bytes = a + b·MP (enc) |
|---|---|---|---|
| lodepng | 70 | 0.0401 + 23.922·MP | 2122 + 353781·MP |
| png_balanced | 70 | 0.0579 + 11.880·MP | 2380 + 458254·MP |
| png_fast | 70 | 0.0017 + 2.472·MP | 2555 + 593988·MP |
| png_high | 70 | -0.0287 + 65.270·MP | 2341 + 381348·MP |
| zenpng_e10_st | 70 | 0.0860 + 53.448·MP | 2390 + 297364·MP |
| zenpng_e11_st | 70 | 0.0797 + 56.349·MP | 2391 + 296848·MP |
| zenpng_e12_st | 70 | 0.1489 + 72.774·MP | 2391 + 296513·MP |
| zenpng_e13_mt | 70 | 1.3698 + 100.679·MP | 2408 + 279663·MP |
| zenpng_e13_st | 70 | 0.3718 + 233.260·MP | 2411 + 278803·MP |
| zenpng_e14_st | 70 | 0.2435 + 280.083·MP | 2414 + 276739·MP |
| zenpng_e15_st | 70 | 0.3147 + 407.362·MP | 2404 + 273800·MP |
| zenpng_e16_st | 70 | 1.8570 + 923.089·MP | 2403 + 273732·MP |
| zenpng_e17_st | 70 | 1.7769 + 1360.360·MP | 2407 + 271622·MP |
| zenpng_e18_st | 70 | 1.7668 + 1812.397·MP | 2409 + 270477·MP |
| zenpng_e19_st | 70 | 3.1401 + 2450.500·MP | 2408 + 270468·MP |
| zenpng_e1_mt | 70 | 0.0226 + 2.346·MP | 2437 + 381333·MP |
| zenpng_e1_st | 70 | 0.0109 + 2.300·MP | 2442 + 379446·MP |
| zenpng_e2_st | 70 | 0.0171 + 9.765·MP | 2377 + 366458·MP |
| zenpng_e3_st | 70 | 0.0186 + 13.294·MP | 2361 + 354327·MP |
| zenpng_e4_st | 70 | 0.0265 + 14.138·MP | 2341 + 351944·MP |
| zenpng_e5_st | 70 | 0.0270 + 14.784·MP | 2334 + 349883·MP |
| zenpng_e6_st | 70 | 0.0583 + 30.642·MP | 2344 + 330086·MP |
| zenpng_e7_mt | 70 | 0.5232 + 14.784·MP | 2337 + 328046·MP |
| zenpng_e7_st | 70 | 0.0575 + 34.368·MP | 2337 + 328139·MP |
| zenpng_e8_st | 70 | 0.0678 + 45.680·MP | 2396 + 299052·MP |
| zenpng_e9_st | 70 | 0.0900 + 48.549·MP | 2392 + 298494·MP |
| zune | 70 | -0.0017 + 2.187·MP | -237 + 3131629·MP |

### enc by size_class: median time / png_high time, median size / png_high size

**medium** (n=25)

| arm | time | size | dominated by |
|---|---|---|---|
| zenpng_e1_mt | 0.017 | 1.1445 |  |
| zune | 0.018 | 3.7228 |  |
| zenpng_e1_st | 0.019 | 1.1516 |  |
| png_fast | 0.023 | 1.2117 |  |
| zenpng_e2_st | 0.079 | 1.1088 |  |
| zenpng_e3_st | 0.136 | 1.0923 |  |
| png_balanced | 0.158 | 1.0309 |  |
| zenpng_e5_st | 0.169 | 1.0542 | png_balanced |
| zenpng_e4_st | 0.170 | 1.0750 | png_balanced |
| zenpng_e7_mt | 0.179 | 1.0214 |  |
| lodepng | 0.304 | 1.0142 |  |
| zenpng_e6_st | 0.442 | 1.0357 | lodepng, png_balanced |
| zenpng_e7_st | 0.491 | 1.0194 | lodepng |
| zenpng_e8_st | 0.625 | 1.0081 |  |
| zenpng_e9_st | 0.665 | 1.0065 |  |
| zenpng_e10_st | 0.775 | 1.0039 |  |
| zenpng_e11_st | 0.845 | 1.0019 |  |
| zenpng_e13_mt | 0.981 | 0.9646 |  |
| png_high | 1.000 | 1.0000 |  |
| zenpng_e12_st | 1.072 | 1.0019 | png_high |
| zenpng_e13_st | 2.982 | 0.9622 |  |
| zenpng_e14_st | 3.053 | 0.9568 |  |
| zenpng_e15_st | 4.175 | 0.9514 |  |
| zenpng_e16_st | 8.315 | 0.9415 |  |
| zenpng_e17_st | 12.570 | 0.9415 |  |
| zenpng_e18_st | 15.929 | 0.9415 |  |
| zenpng_e19_st | 22.482 | 0.9410 |  |

**small** (n=25)

| arm | time | size | dominated by |
|---|---|---|---|
| zune | 0.020 | 3.0251 |  |
| zenpng_e1_st | 0.030 | 1.1393 |  |
| png_fast | 0.034 | 1.1826 |  |
| zenpng_e1_mt | 0.040 | 1.1393 |  |
| zenpng_e2_st | 0.104 | 1.1020 |  |
| zenpng_e3_st | 0.161 | 1.0769 |  |
| zenpng_e4_st | 0.185 | 1.0567 |  |
| zenpng_e5_st | 0.208 | 1.0557 |  |
| png_balanced | 0.230 | 1.0463 |  |
| lodepng | 0.371 | 1.0234 |  |
| zenpng_e6_st | 0.466 | 1.0277 | lodepng |
| zenpng_e7_st | 0.554 | 1.0214 |  |
| zenpng_e7_mt | 0.672 | 1.0214 |  |
| zenpng_e8_st | 0.770 | 1.0196 |  |
| zenpng_e9_st | 0.796 | 1.0189 |  |
| zenpng_e10_st | 0.886 | 1.0162 |  |
| zenpng_e11_st | 0.936 | 1.0162 |  |
| png_high | 1.000 | 1.0000 |  |
| zenpng_e12_st | 1.238 | 1.0066 | png_high |
| zenpng_e13_mt | 3.042 | 0.9607 |  |
| zenpng_e13_st | 3.231 | 0.9607 |  |
| zenpng_e14_st | 3.449 | 0.9597 |  |
| zenpng_e15_st | 4.689 | 0.9541 |  |
| zenpng_e16_st | 11.030 | 0.9468 |  |
| zenpng_e17_st | 14.113 | 0.9468 |  |
| zenpng_e18_st | 18.843 | 0.9468 |  |
| zenpng_e19_st | 25.734 | 0.9466 |  |

**tiny** (n=20)

| arm | time | size | dominated by |
|---|---|---|---|
| zune | 0.025 | 2.1312 |  |
| png_fast | 0.051 | 1.1223 |  |
| zenpng_e1_st | 0.097 | 1.0831 |  |
| zenpng_e1_mt | 0.156 | 1.0831 |  |
| zenpng_e2_st | 0.256 | 1.0569 |  |
| zenpng_e3_st | 0.342 | 1.0363 |  |
| zenpng_e4_st | 0.434 | 1.0242 |  |
| zenpng_e5_st | 0.456 | 1.0171 |  |
| png_balanced | 0.541 | 1.0183 |  |
| lodepng | 0.691 | 1.0059 |  |
| png_high | 1.000 | 1.0000 |  |
| zenpng_e6_st | 1.048 | 0.9992 |  |
| zenpng_e7_st | 1.110 | 0.9998 |  |
| zenpng_e8_st | 1.404 | 0.9997 |  |
| zenpng_e9_st | 1.576 | 0.9986 |  |
| zenpng_e10_st | 1.666 | 0.9970 |  |
| zenpng_e11_st | 1.715 | 0.9969 |  |
| zenpng_e12_st | 2.550 | 0.9963 |  |
| zenpng_e7_mt | 3.284 | 0.9998 |  |
| zenpng_e13_st | 6.850 | 0.9785 |  |
| zenpng_e14_st | 6.853 | 0.9785 |  |
| zenpng_e13_mt | 9.626 | 0.9785 |  |
| zenpng_e15_st | 10.121 | 0.9750 |  |
| zenpng_e16_st | 27.908 | 0.9697 |  |
| zenpng_e17_st | 35.190 | 0.9697 |  |
| zenpng_e18_st | 44.795 | 0.9697 |  |
| zenpng_e19_st | 64.105 | 0.9674 |  |

### enc by content: median time / png_high time, median size / png_high size

**document** (n=16)

| arm | time | size | dominated by |
|---|---|---|---|
| zune | 0.023 | 3.8239 |  |
| png_fast | 0.033 | 1.1997 |  |
| zenpng_e1_st | 0.035 | 1.1456 |  |
| zenpng_e1_mt | 0.037 | 1.1478 |  |
| zenpng_e2_st | 0.141 | 1.1045 |  |
| zenpng_e3_st | 0.200 | 1.0772 |  |
| zenpng_e4_st | 0.218 | 1.0651 |  |
| zenpng_e5_st | 0.235 | 1.0603 |  |
| png_balanced | 0.249 | 1.0562 |  |
| lodepng | 0.412 | 1.0491 |  |
| zenpng_e6_st | 0.506 | 1.0327 |  |
| zenpng_e7_st | 0.621 | 1.0266 |  |
| zenpng_e7_mt | 0.636 | 1.0269 |  |
| zenpng_e8_st | 0.856 | 1.0065 |  |
| zenpng_e9_st | 0.899 | 1.0049 |  |
| zenpng_e10_st | 0.961 | 1.0046 |  |
| zenpng_e11_st | 0.982 | 1.0046 |  |
| png_high | 1.000 | 1.0000 |  |
| zenpng_e12_st | 1.247 | 0.9994 |  |
| zenpng_e13_mt | 2.666 | 0.9689 |  |
| zenpng_e13_st | 3.362 | 0.9660 |  |
| zenpng_e14_st | 3.985 | 0.9625 |  |
| zenpng_e15_st | 7.077 | 0.9554 |  |
| zenpng_e16_st | 15.982 | 0.9432 |  |
| zenpng_e17_st | 25.387 | 0.9420 |  |
| zenpng_e18_st | 36.531 | 0.9420 |  |
| zenpng_e19_st | 45.669 | 0.9408 |  |

**lineart** (n=11)

| arm | time | size | dominated by |
|---|---|---|---|
| zune | 0.022 | 6.6755 |  |
| png_fast | 0.031 | 1.4790 |  |
| zenpng_e1_mt | 0.043 | 1.0957 |  |
| zenpng_e1_st | 0.047 | 1.0957 |  |
| zenpng_e2_st | 0.113 | 1.0677 |  |
| zenpng_e3_st | 0.161 | 1.0677 |  |
| zenpng_e4_st | 0.177 | 1.0578 |  |
| zenpng_e5_st | 0.186 | 1.0567 |  |
| png_balanced | 0.199 | 1.0732 |  |
| lodepng | 0.324 | 1.0436 |  |
| zenpng_e6_st | 0.411 | 1.0418 |  |
| zenpng_e7_mt | 0.475 | 1.0362 |  |
| zenpng_e7_st | 0.502 | 1.0329 |  |
| zenpng_e8_st | 0.689 | 1.0221 |  |
| zenpng_e9_st | 0.768 | 1.0172 |  |
| zenpng_e10_st | 0.886 | 1.0112 |  |
| zenpng_e11_st | 0.936 | 1.0117 |  |
| png_high | 1.000 | 1.0000 |  |
| zenpng_e12_st | 1.156 | 0.9992 |  |
| zenpng_e13_mt | 2.270 | 0.9452 |  |
| zenpng_e13_st | 3.231 | 0.9452 |  |
| zenpng_e14_st | 3.505 | 0.9439 |  |
| zenpng_e15_st | 5.423 | 0.9417 |  |
| zenpng_e16_st | 12.397 | 0.9309 |  |
| zenpng_e17_st | 17.561 | 0.9309 |  |
| zenpng_e18_st | 25.857 | 0.9309 |  |
| zenpng_e19_st | 31.428 | 0.9309 |  |

**mixed** (n=9)

| arm | time | size | dominated by |
|---|---|---|---|
| zune | 0.027 | 1.5171 |  |
| zenpng_e1_st | 0.034 | 1.0652 |  |
| zenpng_e1_mt | 0.044 | 1.0652 |  |
| png_fast | 0.060 | 1.1107 |  |
| zenpng_e2_st | 0.158 | 1.0569 |  |
| zenpng_e3_st | 0.205 | 1.0338 |  |
| zenpng_e4_st | 0.257 | 1.0219 |  |
| zenpng_e5_st | 0.273 | 1.0144 |  |
| png_balanced | 0.595 | 1.0084 |  |
| lodepng | 0.694 | 1.0020 |  |
| zenpng_e6_st | 0.855 | 0.9978 |  |
| zenpng_e7_st | 0.876 | 0.9991 |  |
| png_high | 1.000 | 1.0000 |  |
| zenpng_e8_st | 1.020 | 0.9991 |  |
| zenpng_e7_mt | 1.065 | 0.9991 |  |
| zenpng_e9_st | 1.084 | 0.9986 |  |
| zenpng_e10_st | 1.184 | 0.9986 |  |
| zenpng_e11_st | 1.218 | 0.9986 |  |
| zenpng_e12_st | 2.518 | 0.9978 |  |
| zenpng_e13_mt | 3.363 | 0.9771 |  |
| zenpng_e13_st | 4.109 | 0.9739 |  |
| zenpng_e14_st | 4.115 | 0.9738 |  |
| zenpng_e15_st | 4.715 | 0.9737 |  |
| zenpng_e16_st | 12.016 | 0.9688 |  |
| zenpng_e17_st | 15.136 | 0.9688 |  |
| zenpng_e18_st | 18.843 | 0.9688 |  |
| zenpng_e19_st | 27.569 | 0.9679 |  |

**photo** (n=26)

| arm | time | size | dominated by |
|---|---|---|---|
| zune | 0.017 | 2.3168 |  |
| zenpng_e1_st | 0.022 | 1.1203 |  |
| zenpng_e1_mt | 0.029 | 1.1170 |  |
| png_fast | 0.031 | 1.1197 |  |
| zenpng_e2_st | 0.092 | 1.0846 |  |
| zenpng_e3_st | 0.153 | 1.0587 |  |
| zenpng_e4_st | 0.231 | 1.0414 |  |
| zenpng_e5_st | 0.253 | 1.0302 |  |
| png_balanced | 0.277 | 1.0122 |  |
| lodepng | 0.478 | 0.9980 |  |
| zenpng_e7_mt | 0.493 | 1.0073 | lodepng |
| zenpng_e6_st | 0.586 | 1.0163 | lodepng, png_balanced |
| zenpng_e7_st | 0.710 | 1.0066 | lodepng |
| zenpng_e8_st | 0.888 | 1.0066 | lodepng |
| zenpng_e9_st | 0.977 | 1.0032 | lodepng |
| png_high | 1.000 | 1.0000 |  |
| zenpng_e10_st | 1.054 | 0.9999 | lodepng |
| zenpng_e11_st | 1.082 | 0.9995 | lodepng |
| zenpng_e12_st | 1.626 | 0.9995 | lodepng |
| zenpng_e13_mt | 2.564 | 0.9711 |  |
| zenpng_e13_st | 3.530 | 0.9700 |  |
| zenpng_e14_st | 3.652 | 0.9700 |  |
| zenpng_e15_st | 4.580 | 0.9661 |  |
| zenpng_e16_st | 10.161 | 0.9572 |  |
| zenpng_e17_st | 13.719 | 0.9572 |  |
| zenpng_e18_st | 18.219 | 0.9572 |  |
| zenpng_e19_st | 24.524 | 0.9566 |  |

**screen** (n=8)

| arm | time | size | dominated by |
|---|---|---|---|
| zune | 0.025 | 6.3333 |  |
| png_fast | 0.035 | 1.4393 |  |
| zenpng_e1_st | 0.040 | 1.3201 |  |
| zenpng_e1_mt | 0.051 | 1.3152 |  |
| zenpng_e2_st | 0.159 | 1.2756 |  |
| zenpng_e3_st | 0.208 | 1.1744 |  |
| png_balanced | 0.215 | 1.0955 |  |
| zenpng_e4_st | 0.232 | 1.1573 | png_balanced |
| zenpng_e5_st | 0.242 | 1.1362 | png_balanced |
| lodepng | 0.372 | 1.0813 |  |
| zenpng_e6_st | 0.511 | 1.0403 |  |
| zenpng_e7_st | 0.591 | 1.0313 |  |
| zenpng_e7_mt | 0.633 | 1.0334 |  |
| zenpng_e8_st | 0.832 | 1.0241 |  |
| zenpng_e9_st | 0.890 | 1.0232 |  |
| zenpng_e10_st | 0.980 | 1.0157 |  |
| png_high | 1.000 | 1.0000 |  |
| zenpng_e11_st | 1.027 | 1.0136 | png_high |
| zenpng_e12_st | 1.272 | 1.0136 | png_high |
| zenpng_e13_mt | 3.385 | 0.9775 |  |
| zenpng_e13_st | 4.502 | 0.9665 |  |
| zenpng_e14_st | 5.037 | 0.9632 |  |
| zenpng_e15_st | 8.594 | 0.9500 |  |
| zenpng_e16_st | 20.129 | 0.9362 |  |
| zenpng_e17_st | 31.864 | 0.9362 |  |
| zenpng_e18_st | 43.049 | 0.9362 |  |
| zenpng_e19_st | 56.189 | 0.9334 |  |

