# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic
Versioning](https://semver.org/spec/v2.0.0.html).

## [0.2.0] - 2023-01-17

### Added

- Added polyphony support. You can now play up to eight voices at once.
- Added an option to crossfade the recorded buffer to avoid or dial down clicks
  when looping.
- Added attack and release time controls to avoid or dial down clicks when
  pressing or releasing a key.
- Added an optional velocity sensitive mode.
- Added support for polyphonic CLAP and VST3 volume note expressions to
  precisely control each note's volume while it's playing.

### Removed

- The normalization option been removed since the old method to automatically
  normalize the buffer doesn't work anymore with recording change mentioned
  below.

### Changed

- Buffr Glitch now starts recording when a note is held down instead of playing
  back previously played audio. This makes it possible to use Buffr Glitch in a
  more rhythmic way without manually offsetting notes. This is particularly
  important at the start of the playback since then the buffer will have
  otherwise been completely silent, and also when playing chords.
