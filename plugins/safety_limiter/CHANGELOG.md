# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic
Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Safety Limiter now logs occurrences of NaN and infinite values so they're
  easier to spot. These values already caused Safety Limiter to engage, but this
  makes it very easy to notice that something fishy is going on during
  development.
