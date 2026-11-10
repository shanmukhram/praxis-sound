# ADR-0003: DSP library: in-house, not fundsp

Status: accepted

fundsp 0.23 (MIT/Apache, last release Jan 2026) was the candidate. Decision: write the DSP
in-house in `praxis-dsp`, because:
- bit-identical determinism and allocation-free processing must be guaranteed and tested by us;
- the engine needs a narrow, explicit `Processor` trait with indexed parameters, tails and latency
  reporting, which fundsp's graph model does not expose in this form;
- the needed set (BLEP oscillators, SVF, RBJ biquads, FDN reverb, partitioned convolution,
  compressor, true-peak limiter, BS.1770 meter) is well understood and modest in size.
FFT comes from `realfft`/`rustfft`; resampling of decoded files from `rubato`.
