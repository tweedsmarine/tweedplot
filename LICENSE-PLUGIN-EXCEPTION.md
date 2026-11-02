# TweedPlot plugin exception

*Additional permission under section 7 of the GNU General Public License, version 3.*

TweedPlot is licensed under the GNU GPL version 3 or later (see [LICENSE](LICENSE)), with
the following additional permission from its copyright holder, Chris Harris (TweedsMarine).

1. **Definitions.**
   - The **plugin interface** is the set of messages defined in the `plugin-api` crate of
     this repository, exchanged between TweedPlot and a separate program over that
     program's standard input and output, together with the plugin package format defined
     there.
   - A **plugin** is a separate, independently compiled program that TweedPlot starts and
     that communicates with TweedPlot only through the plugin interface.

2. **Permission.** You may run, copy and distribute TweedPlot together with, alongside, or
   in a way that starts and uses any plugin, under any licence the plugin's authors choose,
   and that combination does not make the plugin subject to the GNU GPL. This includes
   distributing TweedPlot and a plugin on the same medium or download, and TweedPlot
   offering to download and install a plugin.

3. **Limits.** This permission does not cover code copied from TweedPlot's GPL-licensed
   parts into a plugin, or a plugin that links TweedPlot's GPL-licensed code into its own
   program. The `plugin-api` and `enc-s57` crates are licensed separately under the MIT or
   Apache-2.0 licences, so a plugin may use them under those terms.

4. **Modified versions.** If you modify TweedPlot, you may extend this permission to your
   version, but you are not obliged to; you may remove it from your version.

The TweedPlot S-63 Charts plugin, which handles encrypted (S-63) charts under the IHO's
data protection scheme, is such a plugin: it is TweedsMarine's own, separately licensed
program, and not part of TweedPlot.
