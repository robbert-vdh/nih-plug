/**
 * AU v2 CocoaUI view factory — ObjC shim.
 *
 * `define_class!` (objc2/Rust) does NOT emit __OBJC_CLASS_PROTOCOLS metadata,
 * so hosts reject the factory via `conformsToProtocol:` before ever calling
 * `interfaceVersion` or `uiViewForAudioUnit:withSize:`.
 *
 * Compiling @interface ... : NSObject <AUCocoaUIBase> in real ObjC generates
 * the protocol-conformance metadata that hosts require.
 *
 * DESIGN: The host calls [[ClassName alloc] init] independently — we cannot
 * store per-instance state before `uiViewForAudioUnit:withSize:` fires.
 * Instead the pending spawn closure is stored in a global C variable
 * (nih_plug_au_pending_spawn_raw) set by wrapper.rs before returning the
 * CocoaViewInfo.  The factory's `uiViewForAudioUnit:withSize:` atomically
 * swaps it out (returns NULL on concurrent double-open).
 *
 * Class name injected at compile time (-DNIH_PLUG_AU_VIEW_CLASS=...) to avoid
 * ObjC class-name collisions when multiple nih-plug plugins are loaded in the
 * same host process.
 */

@import AppKit;
@import AudioToolbox;
#import <AudioUnit/AUCocoaUIView.h>
#include <stdatomic.h>

/* Provided by wrapper.rs via extern "C" */
extern void nih_plug_au_cocoaui_spawn(void *parent_ns_view, void *spawn_raw);
extern void nih_plug_au_cocoaui_close_view(void *container_ns_view);

/*
 * Global pending spawn pointer — set by wrapper.rs immediately before
 * returning AUCocoaViewInfo.  The factory atomically swaps it to NULL
 * when `uiViewForAudioUnit:withSize:` is called.
 */
_Atomic(void *) nih_plug_au_pending_spawn_raw = 0;

/*
 * Plugin's preferred editor size (logical points), written by wrapper.rs
 * before storing the spawn pointer.  Read by uiViewForAudioUnit:withSize:
 * to size the container correctly regardless of what the host passes as
 * preferredSize (Ableton Live passes 0×0 on first open).
 */
_Atomic(uint32_t) nih_plug_au_editor_width  = 0;
_Atomic(uint32_t) nih_plug_au_editor_height = 0;

#ifndef NIH_PLUG_AU_VIEW_CLASS
#define NIH_PLUG_AU_VIEW_CLASS NihPlugAuViewFactory
#endif

/*
 * Global strong reference to the container view.
 *
 * Without this, the baseview autorelease-pool drain (inside open_parented)
 * fires immediately after nih_plug_au_cocoaui_spawn returns, releasing the
 * container before Ableton has a chance to retain the returned NSView.
 * The strong reference is cleared when the Rust side calls
 * nih_plug_au_release_container (from close_view).
 */
static NSView *g_containerView = nil;
static NSObject *g_containerLock = nil;

__attribute__((constructor))
static void _init_container_lock(void) {
    g_containerLock = [NSObject new];
}

/* ── Container NSView — overrides dealloc to drop the Rust editor handle ── */

@interface NihPlugAuContainerView : NSView
@end

@implementation NihPlugAuContainerView

- (void)dealloc {
    NSLog(@"[nih-plug AU] NihPlugAuContainerView dealloc: %p", (__bridge void *)self);
    nih_plug_au_cocoaui_close_view((__bridge void *)self);
}

@end

/* ── Factory NSObject conforming to AUCocoaUIBase ───────────────────────── */

@interface NIH_PLUG_AU_VIEW_CLASS : NSObject <AUCocoaUIBase>
@end

@implementation NIH_PLUG_AU_VIEW_CLASS

- (unsigned)interfaceVersion {
    NSLog(@"[nih-plug AU] interfaceVersion called");
    return 0;
}

- (NSView *)uiViewForAudioUnit:(AudioUnit)au withSize:(NSSize)preferredSize {
    NSLog(@"[nih-plug AU] uiViewForAudioUnit:withSize: au=%p size=%.0fx%.0f",
          (void *)au, preferredSize.width, preferredSize.height);

    /* Atomically take the pending spawn closure — prevents double-open. */
    void *spawn_raw = atomic_exchange(&nih_plug_au_pending_spawn_raw, (void *)NULL);
    if (!spawn_raw) {
        NSLog(@"[nih-plug AU] uiViewForAudioUnit: no pending spawn (already consumed or none set)");
        return nil;
    }

    /* Use the plugin's own declared size; fall back to what the host suggests,
     * then to a safe default.  Ableton Live passes preferredSize = {0,0}. */
    uint32_t ew = atomic_load(&nih_plug_au_editor_width);
    uint32_t eh = atomic_load(&nih_plug_au_editor_height);
    CGFloat w = ew > 0 ? (CGFloat)ew
              : (preferredSize.width  > 0 ? preferredSize.width  : 800);
    CGFloat h = eh > 0 ? (CGFloat)eh
              : (preferredSize.height > 0 ? preferredSize.height : 600);
    NSLog(@"[nih-plug AU] uiViewForAudioUnit: using size=%.0fx%.0f (plugin=%ux%u preferred=%.0fx%.0f)",
          w, h, ew, eh, preferredSize.width, preferredSize.height);
    NSRect frame = NSMakeRect(0, 0, w, h);

    NihPlugAuContainerView *container = [[NihPlugAuContainerView alloc] initWithFrame:frame];
    if (!container) {
        NSLog(@"[nih-plug AU] uiViewForAudioUnit: container alloc failed — leaking spawn closure");
        return nil;
    }

    /*
     * Pin the container in a global strong ref before calling into Rust.
     * baseview's open_parented drains its own NSAutoreleasePool, which can
     * trigger a premature release of autorelease-registered objects.
     * The strong ref keeps the container alive until Ableton retains it AND
     * until the Rust side explicitly releases it via nih_plug_au_release_container.
     */
    @synchronized(g_containerLock) {
        if (g_containerView) {
            /* Stale container from a previous open that was not cleanly closed. */
            NSLog(@"[nih-plug AU] uiViewForAudioUnit: replacing stale container=%p",
                  (__bridge void *)g_containerView);
            nih_plug_au_cocoaui_close_view((__bridge void *)g_containerView);
        }
        g_containerView = container;
    }

    NSLog(@"[nih-plug AU] uiViewForAudioUnit: container=%p, spawning editor", (__bridge void *)container);
    nih_plug_au_cocoaui_spawn((__bridge void *)container, spawn_raw);
    NSLog(@"[nih-plug AU] uiViewForAudioUnit: done, returning container");
    return container;
}

@end

/*
 * Called by Rust's close_view to drop the global strong reference.
 * Once this returns, the container's lifetime is entirely in the host's hands.
 */
void nih_plug_au_release_container(void *container_ns_view) {
    @synchronized(g_containerLock) {
        if (g_containerView == (__bridge NihPlugAuContainerView *)container_ns_view) {
            g_containerView = nil;
        }
    }
}
