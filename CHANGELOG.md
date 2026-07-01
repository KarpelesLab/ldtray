# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.1](https://github.com/KarpelesLab/ldtray/compare/v0.1.0...v0.1.1) - 2026-07-01

### Fixed

- *(macos)* allow non_camel_case_types for objc `id`/`SEL` aliases

### Other

- note runtime smoke validation on Windows/macOS CI runners
- *(ci)* run a real tray on the Windows/macOS GUI runners
- mark all three backends implemented in README status table
- macOS backend (NSStatusItem via the Objective-C runtime)
- Windows backend (Shell_NotifyIcon + hidden message window)
