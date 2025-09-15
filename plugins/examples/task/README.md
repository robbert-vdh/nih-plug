# Background task execution example

## Build and bundle the VST and CLAP plugins

```shell
$ cargo xtask bundle task --release
```

## One option is to run in Bitwig Studio on Mac

```
$ NIH_LOG=/tmp/nih.log open /Applications/Bitwig\ Studio.app
$ tail -f /tmp/nih.log
```

### in Bitwig Studio

* Show Browser Panel -> File Browser -> Add Plug-in Location
* Settings -> Plug-ins -> Per Plug-in Overrides
* add the "nih-plug task example" plugin to a track

### tail output

```
11:54:31 [INFO] initialize: initializing the plugin
11:54:33 [INFO] task: task run from initialize method
11:54:33 [INFO] initialize: sent from task
```

* press play on the transport in the host/DAW

```
11:54:45 [INFO] process: processing first buffer after play pressed
11:54:47 [INFO] task: task run from process method
11:54:47 [INFO] process: sent from task
11:54:49 [INFO] task: task run from process method
11:54:49 [INFO] process: sent from task
```
