2026-02-22T23:49:00-07:00 - User: asked for faster adaptive strategies + buffer reuse. Result: 2-3x screening speedup via precomputed filters, sparse BigEnt/Bigrams tracking, removing BigEnt from FAST_STRATEGIES.
2026-02-23T00:00:00-07:00 - User: ready up for new features, update docs. Result: committed monotonicity safety net (fallback_screen_effort, pinned screen@7, boundary-spanning refine_efforts), updated CLAUDE.md with comprehensive effort system docs.
2026-02-23T12:00:00-07:00 - User: implement 6-way dispose/blend optimization for APNG. Result: implemented greedy 1-step lookahead optimizer for truecolor and indexed paths, fixed canvas divergence bug (compress_filtered RGB zeroing), fixed OVER subframe correctness for unchanged pixels. All tests pass.
2026-02-23T18:00:00-07:00 - User: build the world's best PNG optimizer. Comprehensive competitive analysis done (oxipng, pngquant, ECT, zopflipng, APNG Assembler). Identified 12 gaps/opportunities across lossless optimization, lossy, APNG, and frontier research. Implemented all 8 planned improvements:
  1. Color type auto-reduction (RGBA→RGB, RGBA/RGB→Gray, auto-indexing ≤256 colors)
  2. Bit-depth reduction (16→8 when low bytes zero)
  3. Palette luminance sorting for better filter residuals
  4. Dirty transparency in APNG OVER subframes (copy RGB from above for unchanged pixels)
  5. Content-aware brute-force (lower effort threshold for indexed/narrow-row content)
  6. Near-lossless truecolor (LSB quantization, 1-4 bits)
  7. Beam search filter optimization (K-wide incremental DEFLATE state) + block brute-force wiring
  8. Adaptive effort allocation (skip brute-force when filter variance <1%)
2026-02-23T22:30:00-07:00 - Session: fixed beam search + fork brute-force bug. eval_level=1 mapped to Turbo strategy which doesn't support incremental DEFLATE. Changed to eval_level=10 (Greedy) and 15 (Lazy). Fork was silently falling back to filter 0 on every row. Beam search was hitting empty-candidates fallback.
