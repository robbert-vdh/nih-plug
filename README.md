# NIH plugs

Because of course we need to remake everything from scratch!

This is a work in progress JUCE-lite-lite written in Rust to do some experiments
with. The idea is to have a statefull but simple plugin API without too much
magic, while also cutting unnecessary ceremony wherever possible. Since this is
not meant for general use (yet), the plugin API is limited to the functionality
I needed, and I'll expose more functionality as I need it. See the doc comment
in the `Plugin` trait for an incomplete list of missing functionality.

## Licensing

Right now everything is licensed under the GPLv3+ license, partly because the
VST3 bindings used are also GPL licensed. I may split off the VST3 wrapper into
its own crate and relicense the core library under a more permissive license
later.
