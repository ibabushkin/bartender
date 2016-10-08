# `bartender` - a simple I/O multiplexer
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
actions necessary (namely, either spawn a script each `n` seconds or read
linewise from a `FIFO` in the filesystem). These two facilities allow for both
synchronous (timers) and asynchronous (lines coming from a `FIFO`) input that
gets passed to a simple formatting object and printed to `stdout` on updates.

## Examples?
Sure. Here is my `~/.bartenderrc`:
```
format = (
    "%{F#cccccc}%{B#005577}", // tagsets go on the left
    {
        name = "tagset"; // include output of object `tagset` here
    },
    "%{F#657b83}%{B#1c1c1c}%{r}", // the rest goes on the right
    {
        name = "clock"; // include output of object `clock` here
    }
);

tagset = {
    type = "fifo"; // tagsets are read on-demand from a `FIFO`
    fifo_path = "~/tmp/tagset_fifo";
};

clock = {
    type = "timer"; // the time changes once a minute
    seconds = 60;
    sync = true; // sync second invocation to full minute
    command = "~/dotfiles/clock.sh";
};
```

Let's split it up and look how it functions. The config file *has* to define a
list of name `format`. The values inside are of two types: static strings and
objects with a `name` key. For each name key there should be a toplevel object,
specifying all necessary data as shown above.

The configuration syntax is documented
[here](http://codinghighway.com/rust-config/config/).

Now, to actually run bartender, I have this snippet in my `~/.xinitrc`:
```sh
  bartender | lemonbar -p -g 1366x20+0+0 > /dev/null &
  RUST_LOG=debug exec gabelstaplerwm 2> ~/wm_log > ~/tmp/tagset_fifo
```

Granted, this is a pretty minimal configuration, but it serves pretty well for
demonstration purposes. Note that
[`gabelstaplerwm`](https://github.com/ibabushkin/gabelstaplerwm) outputs the tag
data to `stdout` in my configuration.
