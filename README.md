# `bartender` - a simple I/O multiplexer
![Maintenance status badge](https://img.shields.io/maintenance/yes/2022.svg)

`bartender` is a tool to manage the I/O needed to make your bar, such as
[`lemonbar`](https://github.com/LemonBoy/bar), work in an efficient fashion.
Much like a real bartender it handles synchronous and asynchronous events,
while seamlessly updating the data it manages.

## Why?
Many standalone bars expect input from `stdin` and format it according to a
simple set of rules. Updates are done by sending a new line with the changed
data. When a user wants to include the output of both synchronous events, such
as a clock updating every minute, and events that need to be received
asynchronously, he/she is forced to write a shellscript to collect all
necessary data, format it and pipe that to the bar. Most such solutions are
very ad-hoc and often waste lots of resources. `bartender` addresses this
problem by providing a simple way to push all the heavy lifting related to I/O
on a single binary, so that the user can focus on implementing the logic he/she
needs.

## How?
`bartender` reads a simple configuration file from `~/.bartenderrc` or a custom
path passed as a command line parameter, and spawns threads to perform the
actions necessary. Namely:
- spawn a script each `n` seconds
- read linewise from a `FIFO` in the filesystem
- spawn a command and read linewise from its stdout

These facilities allow for both synchronous (timers) and asynchronous (lines
coming from a process or `FIFO`) input that gets passed to a simple formatting
object and printed to `stdout` on updates.

## Examples?
Sure. Here is a `~/.bartenderrc`, which is in TOML format and uses
mustache templates for the output format:
```TOML
# our format string
format = """
{{! our format string }}
{{{ clock }}}
 {{{ calendar }}}
 {{{ fifo_entry }}}
{{^ fifo_entry }} {{! a way of implementing default values in-template }}
some value
{{/ fifo_entry }}
{{mqtt_news}}
 - and some static stuff
"""

[timers.clock]
# you can use `seconds`, `minutes`, `hours` or any combination therof to
# specify the timer interval
seconds = 5
command = "date +%H:%M:%S.%N" # run this command at each interval

[timers.calendar]
hours = 24
command = "date +%F"

[fifos.fifo_entry]
fifo_path = "~/tmp/entry_b_fifo"
default = "some default string" # another way of specifying defaults

[process.mqtt_news]
command = "mosquitto_sub -h host -t news/breaking"
```

Let's split it up and look how it functions. The config file *has* to define a
mustache template in a string of name `format`. The variables are filled from
the timers and FIFOs, as evident above.

Now, to actually run bartender, I have this snippet in my `~/.xinitrc`:
```sh
  bartender | lemonbar -p -g 1366x20+0+0 > /dev/null &
  RUST_LOG=debug exec gabelstaplerwm 2> ~/wm_log > ~/tmp/tagset_fifo
```

Granted, this is a pretty minimal configuration, but it serves pretty well for
demonstration purposes.
