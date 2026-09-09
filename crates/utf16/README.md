# UTF-16 algorithms

Independent safe `no_std + alloc` code-unit search and code-point traversal.
No external dependencies. KMP preprocessing and search are O(pattern + text)
time and O(pattern) auxiliary storage; a shared work counter charges preprocessing,
comparisons and fallback transitions. Forward/reverse queries preserve UTF-16
offsets, overlap and empty-pattern behavior without decoding or replacing units.
Reverse queries scan a bounded prefix once and retain the last match.

Pattern length has an explicit storage limit. Logical limits do not catch
allocator OOM; callers still need an allocator policy. Code-point traversal
preserves lone surrogates and reports them so callers can reject or replace them.
JavaScript conversions, locale rules, normalization and case mapping do not live
here. The jrs adapter follows local ECMA-262 §22.1.3; this core is language-neutral.
