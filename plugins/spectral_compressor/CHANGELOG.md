# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic
Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Added a basic analyzer that visualizes the target curve, the spectral envelope
  followers and gain reduction. The current version will be expanded a bit in
  the future with some tooltips and labels to show more information.

### Changed

- The envelope follower resetting behavior has changed to immediately snap to
  the current value after the plugin is reset. When the plugin resets after
  being suspending or after changing the window size, previously the envelopes
  would be reset to a fixed value. This could result in loud spikes when the
  plugin resumed from suspend when using extreme ratios and threshold settings.
  Resets are now handled much more gracefully.
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
