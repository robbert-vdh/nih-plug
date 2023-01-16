# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic
Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Added an optional velocity sensitive mode.

### Removed

- The normalization option has temporarily been removed since the old method to
  automatically normalize the buffer doesn't work anymore with below recording
  change.

### Changed

- Buffr Glitch now starts recording when a note is held down instead of playing
  back previously played audio. This makes it possible to use Buffr Glitch in a
  more rhythmic way without manually offsetting notes. This is particularly
  important at the start of the playback since then the buffer will have
  otherwise been completely silent.
