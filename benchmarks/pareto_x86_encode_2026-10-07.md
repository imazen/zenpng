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


---

# Final rerun 2026-10-07: zenpng 0908614 (e11-e14 re-spread on the png(19..23) ramp, RGBA8 analysis early exit, top-k screening buffers), zenflate main 1817ce8 (clean). Same host, inputs and command.
# Medium (1024 px) medians vs png High (time / size): e1 0.014 / 1.1516, e7 0.486 / 1.0194, e11 1.076 / 1.0019, e12 2.839 / 0.9672, e13 (Balanced) 3.134 / 0.9587, e15 4.273 / 0.9529, e19 (High) 23.35 / 0.9412.
# e20-e30 (rebuilt in 128ae10, after this build) are not in this run; see CLAUDE.md 'Upper ladder'.

## enc

| arm | n | ms = a + b·MP (a ms, b ms/MP; relative-error fit) | bytes = a + b·MP (enc) |
|---|---|---|---|
| lodepng | 70 | 0.0407 + 24.026·MP | 2123 + 353819·MP |
| png_balanced | 70 | 0.0577 + 11.923·MP | 2380 + 458254·MP |
| png_fast | 70 | 0.0018 + 2.470·MP | 2555 + 594003·MP |
| png_high | 70 | -0.0274 + 65.168·MP | 2341 + 381349·MP |
| zenpng_e10_st | 70 | 0.0845 + 54.007·MP | 2391 + 297427·MP |
| zenpng_e11_st | 70 | 0.1441 + 74.295·MP | 2392 + 296577·MP |
| zenpng_e12_st | 70 | 0.4966 + 189.271·MP | 2404 + 284750·MP |
| zenpng_e13_mt | 70 | 1.3105 + 121.732·MP | 2407 + 279006·MP |
| zenpng_e13_st | 70 | 0.2561 + 287.293·MP | 2410 + 278303·MP |
| zenpng_e14_st | 70 | 0.0733 + 353.701·MP | 2416 + 276121·MP |
| zenpng_e15_st | 70 | 0.3271 + 417.330·MP | 2402 + 274751·MP |
| zenpng_e16_st | 70 | 1.8887 + 940.846·MP | 2400 + 274721·MP |
| zenpng_e17_st | 70 | 1.7871 + 1402.473·MP | 2404 + 272797·MP |
| zenpng_e18_st | 70 | 1.7814 + 1870.776·MP | 2406 + 271549·MP |
| zenpng_e19_st | 70 | 3.1711 + 2512.184·MP | 2404 + 271500·MP |
| zenpng_e1_mt | 70 | 0.0227 + 2.295·MP | 2437 + 381357·MP |
| zenpng_e1_st | 70 | 0.0117 + 2.051·MP | 2443 + 379468·MP |
| zenpng_e2_st | 70 | 0.0181 + 9.554·MP | 2377 + 366486·MP |
| zenpng_e3_st | 70 | 0.0190 + 13.306·MP | 2362 + 354347·MP |
| zenpng_e4_st | 70 | 0.0265 + 13.941·MP | 2348 + 351990·MP |
| zenpng_e5_st | 70 | 0.0269 + 14.673·MP | 2341 + 349909·MP |
| zenpng_e6_st | 70 | 0.0561 + 30.878·MP | 2351 + 330064·MP |
| zenpng_e7_mt | 70 | 0.5205 + 15.150·MP | 2345 + 328025·MP |
| zenpng_e7_st | 70 | 0.0556 + 34.971·MP | 2344 + 328113·MP |
| zenpng_e8_st | 70 | 0.0661 + 46.275·MP | 2396 + 299121·MP |
| zenpng_e9_st | 70 | 0.0896 + 49.512·MP | 2392 + 298562·MP |
| zune | 70 | -0.0016 + 2.123·MP | -241 + 3133123·MP |

### enc by size_class: median time / png_high time, median size / png_high size

**medium** (n=25)

| arm | time | size | dominated by |
|---|---|---|---|
| zenpng_e1_mt | 0.013 | 1.1445 |  |
| zenpng_e1_st | 0.014 | 1.1516 |  |
| zune | 0.015 | 3.7228 |  |
| png_fast | 0.025 | 1.2117 |  |
| zenpng_e2_st | 0.070 | 1.1088 |  |
| zenpng_e3_st | 0.132 | 1.0923 |  |
| png_balanced | 0.157 | 1.0309 |  |
| zenpng_e4_st | 0.159 | 1.0753 | png_balanced |
| zenpng_e5_st | 0.171 | 1.0542 | png_balanced |
| zenpng_e7_mt | 0.183 | 1.0214 |  |
| lodepng | 0.305 | 1.0142 |  |
| zenpng_e6_st | 0.436 | 1.0357 | lodepng, png_balanced |
| zenpng_e7_st | 0.486 | 1.0194 | lodepng |
| zenpng_e8_st | 0.620 | 1.0081 |  |
| zenpng_e9_st | 0.678 | 1.0065 |  |
| zenpng_e10_st | 0.771 | 1.0039 |  |
| zenpng_e13_mt | 0.997 | 0.9634 |  |
| png_high | 1.000 | 1.0000 |  |
| zenpng_e11_st | 1.076 | 1.0019 | png_high |
| zenpng_e12_st | 2.839 | 0.9672 |  |
| zenpng_e14_st | 3.122 | 0.9574 |  |
| zenpng_e13_st | 3.134 | 0.9587 |  |
| zenpng_e15_st | 4.273 | 0.9529 |  |
| zenpng_e16_st | 8.559 | 0.9418 |  |
| zenpng_e17_st | 13.228 | 0.9418 |  |
| zenpng_e18_st | 16.812 | 0.9418 |  |
| zenpng_e19_st | 23.346 | 0.9412 |  |

**small** (n=25)

| arm | time | size | dominated by |
|---|---|---|---|
| zune | 0.019 | 3.0251 |  |
| zenpng_e1_st | 0.026 | 1.1393 |  |
| png_fast | 0.034 | 1.1826 |  |
| zenpng_e1_mt | 0.036 | 1.1393 |  |
| zenpng_e2_st | 0.105 | 1.1020 |  |
| zenpng_e3_st | 0.163 | 1.0769 |  |
| zenpng_e4_st | 0.183 | 1.0577 |  |
| zenpng_e5_st | 0.210 | 1.0557 |  |
| png_balanced | 0.232 | 1.0463 |  |
| lodepng | 0.373 | 1.0234 |  |
| zenpng_e6_st | 0.464 | 1.0277 | lodepng |
| zenpng_e7_st | 0.544 | 1.0214 |  |
| zenpng_e7_mt | 0.676 | 1.0214 |  |
| zenpng_e8_st | 0.771 | 1.0196 |  |
| zenpng_e9_st | 0.799 | 1.0189 |  |
| zenpng_e10_st | 0.888 | 1.0162 |  |
| png_high | 1.000 | 1.0000 |  |
| zenpng_e11_st | 1.232 | 1.0066 | png_high |
| zenpng_e12_st | 2.940 | 0.9723 |  |
| zenpng_e13_mt | 3.242 | 0.9669 |  |
| zenpng_e13_st | 3.401 | 0.9669 |  |
| zenpng_e14_st | 3.624 | 0.9655 |  |
| zenpng_e15_st | 4.681 | 0.9606 |  |
| zenpng_e16_st | 10.952 | 0.9486 |  |
| zenpng_e17_st | 14.394 | 0.9486 |  |
| zenpng_e18_st | 19.609 | 0.9486 |  |
| zenpng_e19_st | 25.528 | 0.9486 |  |

**tiny** (n=20)

| arm | time | size | dominated by |
|---|---|---|---|
| zune | 0.025 | 2.1312 |  |
| png_fast | 0.051 | 1.1223 |  |
| zenpng_e1_st | 0.095 | 1.0831 |  |
| zenpng_e1_mt | 0.152 | 1.0831 |  |
| zenpng_e2_st | 0.252 | 1.0569 |  |
| zenpng_e3_st | 0.341 | 1.0363 |  |
| zenpng_e4_st | 0.429 | 1.0242 |  |
| zenpng_e5_st | 0.453 | 1.0171 |  |
| png_balanced | 0.538 | 1.0183 |  |
| lodepng | 0.701 | 1.0059 |  |
| png_high | 1.000 | 1.0000 |  |
| zenpng_e6_st | 1.019 | 0.9995 |  |
| zenpng_e7_st | 1.097 | 0.9998 |  |
| zenpng_e8_st | 1.393 | 0.9997 |  |
| zenpng_e9_st | 1.553 | 0.9986 |  |
| zenpng_e10_st | 1.659 | 0.9970 |  |
| zenpng_e11_st | 2.499 | 0.9963 |  |
| zenpng_e7_mt | 3.246 | 0.9998 |  |
| zenpng_e12_st | 6.775 | 0.9787 |  |
| zenpng_e13_st | 7.108 | 0.9785 |  |
| zenpng_e14_st | 7.158 | 0.9785 |  |
| zenpng_e13_mt | 9.449 | 0.9785 |  |
| zenpng_e15_st | 10.162 | 0.9750 |  |
| zenpng_e16_st | 28.255 | 0.9697 |  |
| zenpng_e17_st | 36.088 | 0.9697 |  |
| zenpng_e18_st | 43.827 | 0.9697 |  |
| zenpng_e19_st | 64.381 | 0.9674 |  |

### enc by content: median time / png_high time, median size / png_high size

**document** (n=16)

| arm | time | size | dominated by |
|---|---|---|---|
| zune | 0.021 | 3.8239 |  |
| zenpng_e1_st | 0.032 | 1.1456 |  |
| png_fast | 0.033 | 1.1997 |  |
| zenpng_e1_mt | 0.040 | 1.1478 |  |
| zenpng_e2_st | 0.142 | 1.1045 |  |
| zenpng_e3_st | 0.206 | 1.0772 |  |
| zenpng_e4_st | 0.219 | 1.0651 |  |
| zenpng_e5_st | 0.229 | 1.0603 |  |
| png_balanced | 0.249 | 1.0562 |  |
| lodepng | 0.411 | 1.0491 |  |
| zenpng_e6_st | 0.492 | 1.0327 |  |
| zenpng_e7_st | 0.622 | 1.0266 |  |
| zenpng_e7_mt | 0.626 | 1.0269 |  |
| zenpng_e8_st | 0.859 | 1.0065 |  |
| zenpng_e9_st | 0.909 | 1.0049 |  |
| zenpng_e10_st | 0.954 | 1.0046 |  |
| png_high | 1.000 | 1.0000 |  |
| zenpng_e11_st | 1.261 | 0.9994 |  |
| zenpng_e13_mt | 2.874 | 0.9628 |  |
| zenpng_e12_st | 3.204 | 0.9672 |  |
| zenpng_e13_st | 4.376 | 0.9628 |  |
| zenpng_e14_st | 5.651 | 0.9617 |  |
| zenpng_e15_st | 8.082 | 0.9566 |  |
| zenpng_e16_st | 17.748 | 0.9459 |  |
| zenpng_e17_st | 27.978 | 0.9459 |  |
| zenpng_e18_st | 39.502 | 0.9459 |  |
| zenpng_e19_st | 49.807 | 0.9428 |  |

**lineart** (n=11)

| arm | time | size | dominated by |
|---|---|---|---|
| zune | 0.021 | 6.6755 |  |
| png_fast | 0.034 | 1.4790 |  |
| zenpng_e1_mt | 0.037 | 1.0957 |  |
| zenpng_e1_st | 0.042 | 1.0957 |  |
| zenpng_e2_st | 0.113 | 1.0677 |  |
| zenpng_e3_st | 0.163 | 1.0677 |  |
| zenpng_e4_st | 0.176 | 1.0578 |  |
| zenpng_e5_st | 0.185 | 1.0577 |  |
| png_balanced | 0.199 | 1.0732 |  |
| lodepng | 0.325 | 1.0436 |  |
| zenpng_e6_st | 0.414 | 1.0418 |  |
| zenpng_e7_mt | 0.481 | 1.0362 |  |
| zenpng_e7_st | 0.499 | 1.0329 |  |
| zenpng_e8_st | 0.692 | 1.0221 |  |
| zenpng_e9_st | 0.776 | 1.0172 |  |
| zenpng_e10_st | 0.888 | 1.0112 |  |
| png_high | 1.000 | 1.0000 |  |
| zenpng_e11_st | 1.162 | 0.9992 |  |
| zenpng_e13_mt | 2.512 | 0.9474 |  |
| zenpng_e12_st | 2.940 | 0.9907 |  |
| zenpng_e13_st | 3.507 | 0.9474 |  |
| zenpng_e14_st | 3.649 | 0.9445 |  |
| zenpng_e15_st | 5.428 | 0.9423 |  |
| zenpng_e16_st | 13.091 | 0.9376 |  |
| zenpng_e17_st | 18.092 | 0.9334 |  |
| zenpng_e18_st | 26.623 | 0.9318 |  |
| zenpng_e19_st | 32.190 | 0.9318 |  |

**mixed** (n=9)

| arm | time | size | dominated by |
|---|---|---|---|
| zune | 0.027 | 1.5171 |  |
| zenpng_e1_st | 0.033 | 1.0652 |  |
| zenpng_e1_mt | 0.044 | 1.0652 |  |
| png_fast | 0.060 | 1.1107 |  |
| zenpng_e2_st | 0.156 | 1.0569 |  |
| zenpng_e3_st | 0.207 | 1.0338 |  |
| zenpng_e4_st | 0.240 | 1.0219 |  |
| zenpng_e5_st | 0.257 | 1.0144 |  |
| png_balanced | 0.588 | 1.0084 |  |
| lodepng | 0.706 | 1.0020 |  |
| zenpng_e6_st | 0.849 | 0.9978 |  |
| zenpng_e7_st | 0.875 | 0.9991 |  |
| png_high | 1.000 | 1.0000 |  |
| zenpng_e8_st | 1.002 | 0.9991 |  |
| zenpng_e7_mt | 1.056 | 0.9991 |  |
| zenpng_e9_st | 1.064 | 0.9986 |  |
| zenpng_e10_st | 1.164 | 0.9986 |  |
| zenpng_e11_st | 2.508 | 0.9978 |  |
| zenpng_e13_mt | 3.571 | 0.9774 |  |
| zenpng_e12_st | 4.140 | 0.9779 |  |
| zenpng_e14_st | 4.167 | 0.9771 |  |
| zenpng_e13_st | 4.170 | 0.9774 |  |
| zenpng_e15_st | 4.808 | 0.9743 |  |
| zenpng_e16_st | 11.814 | 0.9688 |  |
| zenpng_e17_st | 15.253 | 0.9688 |  |
| zenpng_e18_st | 19.609 | 0.9688 |  |
| zenpng_e19_st | 27.130 | 0.9679 |  |

**photo** (n=26)

| arm | time | size | dominated by |
|---|---|---|---|
| zune | 0.016 | 2.3168 |  |
| zenpng_e1_st | 0.022 | 1.1203 |  |
| zenpng_e1_mt | 0.029 | 1.1170 |  |
| png_fast | 0.031 | 1.1197 |  |
| zenpng_e2_st | 0.089 | 1.0846 |  |
| zenpng_e3_st | 0.150 | 1.0587 |  |
| zenpng_e4_st | 0.222 | 1.0414 |  |
| zenpng_e5_st | 0.241 | 1.0302 |  |
| png_balanced | 0.277 | 1.0122 |  |
| lodepng | 0.477 | 0.9980 |  |
| zenpng_e7_mt | 0.489 | 1.0073 | lodepng |
| zenpng_e6_st | 0.571 | 1.0163 | lodepng, png_balanced |
| zenpng_e7_st | 0.684 | 1.0065 | lodepng |
| zenpng_e8_st | 0.874 | 1.0065 | lodepng |
| zenpng_e9_st | 0.950 | 1.0032 | lodepng |
| png_high | 1.000 | 1.0000 |  |
| zenpng_e10_st | 1.045 | 0.9999 | lodepng |
| zenpng_e11_st | 1.606 | 0.9995 | lodepng |
| zenpng_e13_mt | 2.646 | 0.9721 |  |
| zenpng_e12_st | 3.408 | 0.9729 |  |
| zenpng_e13_st | 3.701 | 0.9697 |  |
| zenpng_e14_st | 3.812 | 0.9697 |  |
| zenpng_e15_st | 4.593 | 0.9665 |  |
| zenpng_e16_st | 10.243 | 0.9603 |  |
| zenpng_e17_st | 14.328 | 0.9603 |  |
| zenpng_e18_st | 18.468 | 0.9603 |  |
| zenpng_e19_st | 25.053 | 0.9600 |  |

**screen** (n=8)

| arm | time | size | dominated by |
|---|---|---|---|
| zune | 0.025 | 6.3333 |  |
| zenpng_e1_st | 0.035 | 1.3201 |  |
| png_fast | 0.035 | 1.4393 |  |
| zenpng_e1_mt | 0.049 | 1.3152 |  |
| zenpng_e2_st | 0.147 | 1.2756 |  |
| zenpng_e3_st | 0.211 | 1.1744 |  |
| png_balanced | 0.213 | 1.0955 |  |
| zenpng_e4_st | 0.230 | 1.1577 | png_balanced |
| zenpng_e5_st | 0.241 | 1.1364 | png_balanced |
| lodepng | 0.375 | 1.0813 |  |
| zenpng_e6_st | 0.507 | 1.0403 |  |
| zenpng_e7_st | 0.584 | 1.0313 |  |
| zenpng_e7_mt | 0.635 | 1.0334 |  |
| zenpng_e8_st | 0.829 | 1.0241 |  |
| zenpng_e9_st | 0.888 | 1.0232 |  |
| zenpng_e10_st | 0.977 | 1.0157 |  |
| png_high | 1.000 | 1.0000 |  |
| zenpng_e11_st | 1.271 | 1.0136 | png_high |
| zenpng_e12_st | 3.284 | 0.9894 |  |
| zenpng_e13_mt | 4.157 | 0.9707 |  |
| zenpng_e13_st | 4.934 | 0.9652 |  |
| zenpng_e14_st | 5.693 | 0.9592 |  |
| zenpng_e15_st | 8.392 | 0.9518 |  |
| zenpng_e16_st | 19.205 | 0.9403 |  |
| zenpng_e17_st | 32.510 | 0.9403 |  |
| zenpng_e18_st | 44.038 | 0.9403 |  |
| zenpng_e19_st | 57.255 | 0.9348 |  |

