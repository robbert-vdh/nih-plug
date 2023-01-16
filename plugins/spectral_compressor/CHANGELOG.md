# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic
Versioning](https://semver.org/spec/v2.0.0.html).

## [0.3.0] - 2023-01-15

### Added

- Added the version number and a link to the GitHub page to the GUI to make it
  easier to determine which version you're using.

### Removed

- The DC filter option is gone. It was used to prevent upwards compression from
  amplifying DC and very low subbass signals in the original Spectral
  Compressor, but this iteration of Spectral Compressor doesn't need it and the
  feature only caused confusion.

### Changed

- The compression part of Spectral Compressor has been rewritten to have
  theoretically smoother and cleaner transfer curves and to be slightly more
  performant.
- The downwards hi-freq rolloff parameter now correctly scales the ratios.
  Previously the parameter didn't do much.
