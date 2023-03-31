# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic
Versioning](https://semver.org/spec/v2.0.0.html).

## [0.4.3] - 2023-03-31

### Changed

- The default window size has been changed to 2048 since this offers a slightly
  better tradeoff between faster timings and more spectral precision. Existing
  instances are not affected.

### Fixed

- Fixed the soft-knee options in the sidechain matching mode. They previously
  didn't account for the changing compressor thresholds, which could result in
  unexpected loud volume spikes.
- The sidechain matching mode now caps the relative thresholds to behave more
  consistently with quiet inputs.

## [0.4.2] - 2023-03-22

### Changed

- Reworked the resetting behavior again to smoothly fade the timings in after a
  reset to allow the envelop followers to more gently settle in.

## [0.4.1] - 2023-03-22

### Fixed

- Fixed a regression from version 0.4.0 that caused the envelope followers to
  always be stuck at their fastest timings.

## [0.4.0] - 2023-03-22

### Added

- Added an analyzer that visualizes the target curve, the spectral envelope
  followers and gain reduction. The current version will be expanded a bit in a
  future with tooltips and labels to show more information.

### Changed

- The envelope followers reset in a smarter way after the plugin resumes from
  sleep or when the window size has changed. This avoids loud spikes in these
  situations when using extreme compression settings and slow timings.
- The default window overlap amount setting has changed to 16x. Existing patches
  are not affected.
- On Windows, clicking on the plugin's name no longer takes you to Spectral
  Compressor's home page. This is a temporary workaround for an issue with an
  underlying library.

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
