#import <AppKit/AppKit.h>
#import <QuartzCore/QuartzCore.h>
#import <Carbon/Carbon.h>
#import <WebKit/WebKit.h>
#import <unistd.h>

/**
 * 顾清影 菜单栏 App
 * - 左键点击状态栏图标弹出菜单（保持习惯）
 * - 「打开面板」在默认浏览器打开 WebUI（http://127.0.0.1:4096），
 *   不再维护独立的 NSPanel/WKWebView 窗口，避免与 WebUI 双份界面冗余
 * - 状态栏图标随状态变化（空闲 sparkles / 备份中 clock）
 * - 菜单含状态区（模型/记忆/备份时间，异步刷新）+ 常用功能
 */
@interface GQYMenuBarDelegate : NSObject <NSApplicationDelegate, NSMenuDelegate>
@property(nonatomic, strong) NSStatusItem *statusItem;
@property(nonatomic, strong) NSTask *webTask;
@property(nonatomic, strong) NSTask *llamaTask;
@property(nonatomic, strong) NSTask *backupTask;
@property(nonatomic, strong) NSMenuItem *backupItem;
@property(nonatomic, strong) NSMenuItem *loginItemMenu;
@property(nonatomic, strong) NSMenuItem *statusModelItem;
@property(nonatomic, strong) NSMenuItem *statusMemoryItem;
@property(nonatomic, strong) NSMenuItem *statusBackupItem;
@property(nonatomic, assign) BOOL backupInProgress;
@property(nonatomic, strong) NSImage *statusItemIcon;
// 双模交互中心：右键运维菜单 + 左键/⌥G 悬浮卡片
@property(nonatomic, strong) NSMenu *mainMenu;
@property(nonatomic, strong) NSPanel *quickPanel;
@property(nonatomic, strong) WKWebView *quickWebView;
@property(nonatomic, assign) BOOL quickPanelLoaded;
@end

// 悬浮卡片面板：可成为 key（接收键盘输入），Esc 收起
@interface GQYQuickPanel : NSPanel
@end

@implementation GQYQuickPanel
- (BOOL)canBecomeKeyWindow {
    return YES;
}
- (void)cancelOperation:(id)sender {
    (void)sender;
    [self orderOut:nil];
}
@end

@implementation GQYMenuBarDelegate

- (void)applicationDidFinishLaunching:(NSNotification *)notification {
    (void)notification;
    [NSApp setActivationPolicy:NSApplicationActivationPolicyAccessory];

    self.statusItem = [[NSStatusBar systemStatusBar]
        statusItemWithLength:NSVariableStatusItemLength];
    // 优先用 App 图标（顾清影头像）作为状态栏图标，加载失败回退 sparkles
    NSImage *appIcon = [NSImage imageNamed:@"AppIcon"];
    if (appIcon) {
        appIcon.size = NSMakeSize(18, 18);
        self.statusItem.button.image = appIcon;
    } else {
        appIcon = [NSImage
            imageWithSystemSymbolName:@"sparkles"
            accessibilityDescription:@"顾清影"];
        self.statusItem.button.image = appIcon;
    }
    self.statusItemIcon = appIcon;
    self.statusItem.button.toolTip = @"顾清影 —— 点开菜单（⌥H 打开 WebUI）";

    NSMenu *menu = [[NSMenu alloc] init];

    // ── 标题行 ──
    NSMenuItem *titleItem = [[NSMenuItem alloc] initWithTitle:@"顾清影"
                                                       action:nil
                                                keyEquivalent:@""];
    NSMutableAttributedString *title = [[NSMutableAttributedString alloc]
        initWithString:@"顾清影"
        attributes:@{
            NSFontAttributeName: [NSFont boldSystemFontOfSize:13],
            NSForegroundColorAttributeName: NSColor.labelColor,
        }];
    NSString *version = NSBundle.mainBundle.infoDictionary[@"CFBundleShortVersionString"];
    if (version.length > 0) {
        [title appendAttributedString:[[NSAttributedString alloc]
            initWithString:[NSString stringWithFormat:@"  v%@", version]
            attributes:@{
                NSFontAttributeName: [NSFont systemFontOfSize:11],
                NSForegroundColorAttributeName: NSColor.secondaryLabelColor,
            }]];
    }
    titleItem.attributedTitle = title;
    titleItem.enabled = NO;
    [menu addItem:titleItem];

    // ── 状态区（异步刷新）──
    self.statusModelItem = [self statusItemWithTitle:@"模型：…"];
    self.statusMemoryItem = [self statusItemWithTitle:@"记忆：…"];
    self.statusBackupItem = [self statusItemWithTitle:@"备份：…"];
    [menu addItem:self.statusModelItem];
    [menu addItem:self.statusMemoryItem];
    [menu addItem:self.statusBackupItem];
    [menu addItem:[NSMenuItem separatorItem]];

    // ── 功能 ──
    NSMenuItem *panelItem = [self itemWithTitle:@"打开 WebUI"
                                         symbol:@"square.grid.2x2"
                                         action:@selector(openWebPanel:)];
    // 菜单内显式标注 ⌥H（全局快捷键由 Carbon RegisterEventHotKey 注册，两者互补）
    panelItem.keyEquivalent = @"h";
    panelItem.keyEquivalentModifierMask = NSEventModifierFlagOption;
    [menu addItem:panelItem];
    [menu addItem:[self itemWithTitle:@"打开配置"
                               symbol:@"gearshape"
                               action:@selector(openConfigPanel:)]];
    [menu addItem:[self itemWithTitle:@"重启面板服务"
                               symbol:@"arrow.clockwise"
                               action:@selector(restartWebServer:)]];
    [menu addItem:[self itemWithTitle:@"打开终端对话"
                               symbol:@"terminal"
                               action:@selector(openTerminalChat:)]];
    [menu addItem:[NSMenuItem separatorItem]];
    self.backupItem = [self itemWithTitle:@"立即备份并推送"
                                   symbol:@"externaldrive.fill.badge.checkmark"
                                   action:@selector(backupNow:)];
    [menu addItem:self.backupItem];
    // 高级与数据子菜单：收敛次要入口，让核心按钮更突出
    NSMenuItem *advancedItem = [[NSMenuItem alloc] initWithTitle:@"高级与数据"
                                                         action:nil
                                                  keyEquivalent:@""];
    NSMenu *advancedMenu = [[NSMenu alloc] init];
    [advancedMenu addItem:[self itemWithTitle:@"打开独立主目录"
                                       symbol:@"folder"
                                       action:@selector(openAssistantHome:)]];
    [advancedMenu addItem:[self itemWithTitle:@"打开配置文件"
                                       symbol:@"doc.text"
                                       action:@selector(openConfigFile:)]];
    advancedItem.submenu = advancedMenu;
    [menu addItem:advancedItem];
    [menu addItem:[NSMenuItem separatorItem]];
    self.loginItemMenu = [self itemWithTitle:@"开机自启"
                                      symbol:@"power"
                                      action:@selector(toggleLoginItem:)];
    [menu addItem:self.loginItemMenu];
    [menu addItem:[NSMenuItem separatorItem]];
    [menu addItem:[self itemWithTitle:@"退出顾清影"
                               symbol:@"xmark.circle"
                               action:@selector(quit:)]];
    self.statusItem.menu = nil; // 双模：左键悬浮卡片，右键菜单（手动弹出）
    self.mainMenu = menu;
    menu.delegate = self;
    self.statusItem.button.target = self;
    self.statusItem.button.action = @selector(statusButtonClicked:);
    [self.statusItem.button sendActionOn:(NSEventMaskLeftMouseUp | NSEventMaskRightMouseUp)];
    [self refreshLoginItemState];
    [self refreshStatus];
    [self registerGlobalHotkey];
    // 本地推理进程跟随菜单栏（非开机自启）；若用户已开启自启则 LaunchAgent 已拉起，这里自动复用
    [self ensureLlamaServer];
}

// 左右键分发：左键 = 悬浮卡片（即问即答），右键 = 运维菜单
- (void)statusButtonClicked:(id)sender {
    (void)sender;
    NSEvent *event = NSApp.currentEvent;
    if (event.type == NSEventTypeRightMouseUp) {
        [self.mainMenu popUpMenuPositioningItem:nil
                                     atLocation:NSEvent.mouseLocation
                                         inView:nil];
    } else {
        [self showQuickPanel:nil];
    }
}

// 全局快捷键：⌥H = 在浏览器打开面板
static EventHotKeyRef g_panel_hotkey_ref = NULL;
static EventHotKeyRef g_quick_hotkey_ref = NULL;
static OSStatus gqy_hotkey_handler(EventHandlerCallRef nextHandler,
                                   EventRef event,
                                   void *userData) {
    (void)nextHandler;
    (void)event;
    GQYMenuBarDelegate *delegate = (__bridge GQYMenuBarDelegate *)userData;
    EventHotKeyID hotkey_id;
    if (GetEventParameter(event, kEventParamDirectObject, typeEventHotKeyID,
                          NULL, sizeof(hotkey_id), NULL, &hotkey_id) == noErr) {
        if (hotkey_id.id == 2) {
            // ⌥H：在默认浏览器打开 WebUI
            [delegate openWebPanel:nil];
            return noErr;
        }
        if (hotkey_id.id == 3) {
            // ⌥G：唤起悬浮卡片（即问即答）
            [delegate showQuickPanel:nil];
            return noErr;
        }
    }
    return noErr;
}

- (void)registerGlobalHotkey {
    EventTypeSpec event_type = { .eventClass = kEventClassKeyboard,
                                 .eventKind = kEventHotKeyPressed };
    InstallEventHandler(GetEventDispatcherTarget(),
                        gqy_hotkey_handler,
                        1,
                        &event_type,
                        (__bridge void *)self,
                        NULL);
    // ⌥H：Option + H → 完整面板
    EventHotKeyID panel_id = { .signature = 'GQYH', .id = 2 };
    RegisterEventHotKey(kVK_ANSI_H, optionKey, panel_id,
                        GetEventDispatcherTarget(), 0, &g_panel_hotkey_ref);
    // ⌥G：Option + G → 悬浮卡片（即问即答）
    EventHotKeyID quick_id = { .signature = 'GQYQ', .id = 3 };
    RegisterEventHotKey(kVK_ANSI_G, optionKey, quick_id,
                        GetEventDispatcherTarget(), 0, &g_quick_hotkey_ref);
}

// 重启面板服务：杀掉占用 4096 端口的全部旧 gqy web 进程（不限自己 spawn 的），
// 立即用当前二进制重新启动，轮询健康检查通过后才提示成功。
- (void)restartWebServer:(id)sender {
    (void)sender;
    if (self.webTask.isRunning) {
        [self.webTask terminate];
    }
    self.webTask = nil;
    [self terminateGqyWebOnPort:4096];
    [self ensureWebServer:^(BOOL ready) {
        dispatch_async(dispatch_get_main_queue(), ^{
            if (ready) {
                [self showInfo:@"面板服务已重启"
                        detail:@"已加载最新版本与配置，点击「打开 WebUI」即可使用。"];
            } else {
                [self showError:[NSError errorWithDomain:@"GQYMenuBar"
                                                    code:2
                                                userInfo:@{
                                                    NSLocalizedDescriptionKey:
                                                        @"面板服务重启失败，请稍后重试。"
                                                }]];
            }
        });
    }];
}

// 找到并终止监听指定端口的 gqy web 进程（可能是旧版 App 自启或手动启动的残留）
- (void)terminateGqyWebOnPort:(uint16_t)port {
    NSTask *task = [[NSTask alloc] init];
    task.executableURL = [NSURL fileURLWithPath:@"/usr/sbin/lsof"];
    task.arguments = @[
        [NSString stringWithFormat:@"-tiTCP:%u", port],
        @"-sTCP:LISTEN",
    ];
    NSPipe *pipe = [NSPipe pipe];
    task.standardOutput = pipe;
    task.standardError = [NSPipe pipe];
    if (![task launchAndReturnError:nil]) {
        return;
    }
    [task waitUntilExit];
    NSData *data = [pipe.fileHandleForReading readDataToEndOfFile];
    NSString *output = [[NSString alloc] initWithData:data encoding:NSUTF8StringEncoding];
    for (NSString *line in [output componentsSeparatedByString:@"\n"]) {
        NSString *pidText = [line stringByTrimmingCharactersInSet:NSCharacterSet.whitespaceCharacterSet];
        if (pidText.length == 0) {
            continue;
        }
        pid_t pid = pidText.intValue;
        if (pid <= 0) {
            continue;
        }
        // 只杀 gqy 进程，避免误伤占用同端口的其他程序
        NSTask *ps = [[NSTask alloc] init];
        ps.executableURL = [NSURL fileURLWithPath:@"/bin/ps"];
        ps.arguments = @[@"-p", pidText, @"-o", @"comm="];
        NSPipe *psPipe = [NSPipe pipe];
        ps.standardOutput = psPipe;
        ps.standardError = [NSPipe pipe];
        if (![ps launchAndReturnError:nil]) {
            continue;
        }
        [ps waitUntilExit];
        NSData *psData = [psPipe.fileHandleForReading readDataToEndOfFile];
        NSString *comm = [[NSString alloc] initWithData:psData encoding:NSUTF8StringEncoding];
        if ([comm rangeOfString:@"gqy"].location == NSNotFound) {
            continue;
        }
        kill(pid, SIGTERM);
    }
    // 等旧进程退出释放端口，避免新进程 bind 失败
    usleep(400 * 1000);
}

- (void)applicationWillTerminate:(NSNotification *)notification {
    (void)notification;
    // 兜底：若 daemon 是本 App 拉起的且 shutdown 未生效，直接终止
    if (self.webTask.isRunning) {
        [self.webTask terminate];
    }
    // 本地推理进程（llama.cpp）跟随菜单栏退出：只杀自己拉起的，不碰外部服务
    if (self.llamaTask.isRunning) {
        [self.llamaTask terminate];
    }
}

- (NSMenuItem *)itemWithTitle:(NSString *)title
                       symbol:(NSString *)symbolName
                       action:(SEL)action {
    NSMenuItem *item = [[NSMenuItem alloc] initWithTitle:title
                                                 action:action
                                          keyEquivalent:@""];
    item.target = self;
    if (symbolName.length > 0) {
        item.image = [NSImage imageWithSystemSymbolName:symbolName
                               accessibilityDescription:title];
        item.image.size = NSMakeSize(15, 15);
    }
    return item;
}

// 状态行：灰色小字、不可点
- (NSMenuItem *)statusItemWithTitle:(NSString *)title {
    NSMenuItem *item = [[NSMenuItem alloc] initWithTitle:title
                                                   action:nil
                                            keyEquivalent:@""];
    NSMutableParagraphStyle *paragraph = [[NSMutableParagraphStyle alloc] init];
    paragraph.headIndent = 18;
    item.attributedTitle = [[NSAttributedString alloc]
        initWithString:title
        attributes:@{
            NSFontAttributeName: [NSFont systemFontOfSize:11],
            NSForegroundColorAttributeName: NSColor.secondaryLabelColor,
            NSParagraphStyleAttributeName: paragraph,
        }];
    item.enabled = NO;
    return item;
}

// 状态项：月青 ● = 正常就绪，淡紫 ● = 记忆/特殊，绿 ● = 备份已同步，灰 ● = 未知
- (void)setStatusItem:(NSMenuItem *)item title:(NSString *)title color:(NSColor *)color {
    NSMutableParagraphStyle *paragraph = [[NSMutableParagraphStyle alloc] init];
    paragraph.headIndent = 18;
    NSMutableAttributedString *attributed = [[NSMutableAttributedString alloc] init];
    [attributed appendAttributedString:[[NSAttributedString alloc]
        initWithString:@"● "
        attributes:@{
            NSFontAttributeName: [NSFont systemFontOfSize:9],
            NSForegroundColorAttributeName: color,
        }]];
    [attributed appendAttributedString:[[NSAttributedString alloc]
        initWithString:title
        attributes:@{
            NSFontAttributeName: [NSFont systemFontOfSize:11],
            NSForegroundColorAttributeName: NSColor.secondaryLabelColor,
            NSParagraphStyleAttributeName: paragraph,
        }]];
    item.attributedTitle = attributed;
}

- (NSURL *)assistantHome {
    NSString *configured = NSProcessInfo.processInfo.environment[@"GQY_HOME"];
    if (configured.length > 0) {
        return [NSURL fileURLWithPath:configured isDirectory:YES].standardizedURL;
    }
    return [[NSFileManager.defaultManager URLsForDirectory:NSApplicationSupportDirectory
                                                 inDomains:NSUserDomainMask].firstObject
        URLByAppendingPathComponent:@"gqy"
                        isDirectory:YES];
}

- (NSURL *)assistantBinary:(NSError **)error {
    NSDictionary<NSString *, NSString *> *environment =
        NSProcessInfo.processInfo.environment;
    NSString *workingDirectory = environment[@"PWD"];
    NSMutableArray<NSString *> *candidates = [NSMutableArray array];
    NSString *bundled = [NSBundle.mainBundle pathForResource:@"gqy" ofType:nil];
    if (bundled.length > 0) {
        [candidates addObject:bundled];
    }
    if (environment[@"GQY_BIN"].length > 0) {
        [candidates addObject:environment[@"GQY_BIN"]];
    }
    [candidates addObjectsFromArray:@[
        @"/opt/homebrew/bin/gqy",
        @"/usr/local/bin/gqy",
    ]];
    if (workingDirectory.length > 0) {
        [candidates addObject:[workingDirectory
                                  stringByAppendingPathComponent:@"target/release/gqy"]];
        [candidates addObject:[workingDirectory
                                  stringByAppendingPathComponent:@"target/debug/gqy"]];
    }
    for (NSString *candidate in candidates) {
        if ([NSFileManager.defaultManager isExecutableFileAtPath:candidate]) {
            return [NSURL fileURLWithPath:candidate];
        }
    }
    if (error) {
        *error = [NSError errorWithDomain:@"GQYMenuBar"
                                     code:1
                                 userInfo:@{
                                     NSLocalizedDescriptionKey:
                                         @"找不到 gqy 后端。请设置 GQY_BIN 为编译后的可执行文件绝对路径。"
                                 }];
    }
    return nil;
}

- (NSTask *)assistantTaskWithArguments:(NSArray<NSString *> *)arguments
                                 error:(NSError **)error {
    NSURL *binary = [self assistantBinary:error];
    if (!binary) {
        return nil;
    }
    NSTask *task = [[NSTask alloc] init];
    task.executableURL = binary;
    task.arguments = arguments;
    NSMutableDictionary<NSString *, NSString *> *environment =
        [NSProcessInfo.processInfo.environment mutableCopy];
    environment[@"GQY_HOME"] = self.assistantHome.path;
    task.environment = environment;
    return task;
}

- (void)openTerminalChat:(id)sender {
    (void)sender;
    NSError *error = nil;
    NSURL *binary = [self assistantBinary:&error];
    if (!binary) {
        [self showError:error];
        return;
    }

    // 平滑化：优先 iTerm2（AppleScript 新建窗口），其次 WezTerm（cli start），
    // 都没有再回退写 .command 弹 Terminal.app
    if ([self launchChatInITerm2:binary]) {
        return;
    }
    if ([self launchChatInWezTerm:binary]) {
        return;
    }
    [self launchChatViaCommandFile:binary];
}

// iTerm2：AppleScript 在当前会话新建窗口直接跑 gqy，免临时文件
- (BOOL)launchChatInITerm2:(NSURL *)binary {
    NSString *shellCmd = [NSString stringWithFormat:
        @"export GQY_HOME=%@; exec %@",
        [self shellQuote:self.assistantHome.path],
        [self shellQuote:binary.path]];
    NSString *source = [NSString stringWithFormat:
        @"tell application \"iTerm\"\n"
        @"  create window with default profile command %@\n"
        @"  activate\n"
        @"end tell",
        [self appleScriptQuote:shellCmd]];
    NSAppleScript *script = [[NSAppleScript alloc] initWithSource:source];
    NSDictionary *execError = nil;
    [script executeAndReturnError:&execError];
    if (execError) {
        return NO; // 未安装 iTerm2 或脚本失败 → 回退
    }
    return YES;
}

// WezTerm：`wezterm start -- <shell命令>` 会启动/复用 GUI 并开新窗
- (BOOL)launchChatInWezTerm:(NSURL *)binary {
    NSArray<NSString *> *candidates = @[
        @"/opt/homebrew/bin/wezterm",
        @"/usr/local/bin/wezterm",
    ];
    NSString *wezterm = nil;
    for (NSString *candidate in candidates) {
        if ([NSFileManager.defaultManager isExecutableFileAtPath:candidate]) {
            wezterm = candidate;
            break;
        }
    }
    if (!wezterm) {
        return NO;
    }
    NSString *shellCmd = [NSString stringWithFormat:
        @"export GQY_HOME=%@; exec %@",
        [self shellQuote:self.assistantHome.path],
        [self shellQuote:binary.path]];
    NSTask *task = [[NSTask alloc] init];
    task.executableURL = [NSURL fileURLWithPath:wezterm];
    task.arguments = @[@"start", @"--", shellCmd];
    NSError *error = nil;
    if (![task launchAndReturnError:&error]) {
        return NO;
    }
    return YES;
}

// 兜底：写 .command 临时文件弹 Terminal.app（历史行为）
- (void)launchChatViaCommandFile:(NSURL *)binary {
    NSError *error = nil;
    NSURL *runtime = [self.assistantHome URLByAppendingPathComponent:@"runtime"
                                                         isDirectory:YES];
    if (![NSFileManager.defaultManager createDirectoryAtURL:runtime
                                withIntermediateDirectories:YES
                                                 attributes:nil
                                                      error:&error]) {
        [self showError:error];
        return;
    }
    NSURL *launcher = [runtime URLByAppendingPathComponent:@"gqy-terminal.command"];
    NSString *script = [NSString stringWithFormat:
        @"#!/bin/zsh\nexport GQY_HOME=%@\nexec %@\n",
        [self shellQuote:self.assistantHome.path],
        [self shellQuote:binary.path]];
    if (![script writeToURL:launcher atomically:YES encoding:NSUTF8StringEncoding error:&error] ||
        ![NSFileManager.defaultManager setAttributes:@{NSFilePosixPermissions: @0700}
                                         ofItemAtPath:launcher.path
                                                error:&error]) {
        [self showError:error];
        return;
    }
    [NSWorkspace.sharedWorkspace openURL:launcher];
}

// AppleScript 字符串字面量转义（\ 与 "）
- (NSString *)appleScriptQuote:(NSString *)value {
    NSString *escaped = [value stringByReplacingOccurrencesOfString:@"\\" withString:@"\\\\"];
    escaped = [escaped stringByReplacingOccurrencesOfString:@"\"" withString:@"\\\""];
    return [NSString stringWithFormat:@"\"%@\"", escaped];
}

// ─────────────────────────── 打开 WebUI（默认浏览器） ───────────────────────────

- (NSURL *)panelURL {
    return [NSURL URLWithString:@"http://127.0.0.1:4096"];
}

- (void)openWebPanel:(id)sender {
    (void)sender;
    [self openPanelWithSettings:NO];
}

// ─────────────────────────── 悬浮卡片（双模交互中心 · 即问即答） ───────────────────────────

// 左键点击托盘图标 / ⌥G：确保 daemon 就绪后弹出悬浮卡片
- (void)showQuickPanel:(id)sender {
    (void)sender;
    [self ensureWebServer:^(BOOL ready) {
        if (!ready) {
            [self showError:[NSError errorWithDomain:@"GQYMenuBar"
                                                code:4
                                            userInfo:@{
                                                NSLocalizedDescriptionKey:
                                                    @"面板服务启动超时，无法打开悬浮卡片。"
                                            }]];
            return;
        }
        dispatch_async(dispatch_get_main_queue(), ^{
            [self presentQuickPanel];
        });
    }];
}

- (void)presentQuickPanel {
    if (!self.quickPanel) {
        GQYQuickPanel *panel = [[GQYQuickPanel alloc]
            initWithContentRect:NSMakeRect(0, 0, 430, 580)
                      styleMask:NSWindowStyleMaskBorderless
                        backing:NSBackingStoreBuffered
                          defer:NO];
        panel.level = NSFloatingWindowLevel;
        panel.collectionBehavior =
            NSWindowCollectionBehaviorCanJoinAllSpaces
            | NSWindowCollectionBehaviorFullScreenAuxiliary
            | NSWindowCollectionBehaviorMoveToActiveSpace;
        panel.hidesOnDeactivate = YES; // 点别处自动收起
        panel.backgroundColor = NSColor.clearColor;
        panel.hasShadow = YES;

        // 毛玻璃圆角卡片内容视图
        NSVisualEffectView *effect =
            [[NSVisualEffectView alloc] initWithFrame:panel.contentView.bounds];
        effect.material = NSVisualEffectMaterialHUDWindow;
        effect.blendingMode = NSVisualEffectBlendingModeBehindWindow;
        effect.state = NSVisualEffectStateActive;
        effect.wantsLayer = YES;
        effect.layer.cornerRadius = 14;
        effect.layer.masksToBounds = YES;
        effect.autoresizingMask = NSViewWidthSizable | NSViewHeightSizable;
        [panel.contentView addSubview:effect];

        WKWebView *web = [[WKWebView alloc]
            initWithFrame:effect.bounds
            configuration:[[WKWebViewConfiguration alloc] init]];
        web.autoresizingMask = NSViewWidthSizable | NSViewHeightSizable;
        [effect addSubview:web];

        self.quickWebView = web;
        self.quickPanel = panel;
    }

    // 定位：主屏右上角、状态栏正下方
    NSScreen *screen = NSScreen.mainScreen ?: NSScreen.screens.firstObject;
    NSRect visible = screen.visibleFrame;
    NSRect frame = self.quickPanel.frame;
    frame.origin.x = NSMaxX(visible) - NSWidth(frame) - 12;
    frame.origin.y = NSMaxY(visible) - NSHeight(frame) - 4;
    [self.quickPanel setFrame:frame display:YES];

    // 先激活 app 再展示面板：首次点击状态栏图标时 app 可能尚未激活，
    // 若先 makeKey 后 activate，面板会瞬间失焦被 hidesOnDeactivate 收起（左键闪退 bug）。
    [NSApp activateIgnoringOtherApps:YES];
    [self.quickPanel makeKeyAndOrderFront:nil];

    // 首次创建加载 panel 模式 WebUI；再次唤起刷新（对话在 daemon 侧持久，刷新不丢上下文）
    if (!self.quickPanelLoaded) {
        self.quickPanelLoaded = YES;
        [self.quickWebView loadRequest:[NSURLRequest
            requestWithURL:[NSURL URLWithString:@"http://127.0.0.1:4096/?panel=1"]]];
    } else {
        [self.quickWebView reload];
    }
}

// 打开 WebUI 并直接展开配置抽屉（等价于终端里的 gqy config，GUI 版）
- (void)openConfigPanel:(id)sender {
    (void)sender;
    [self openPanelWithSettings:YES];
}

- (void)openPanelWithSettings:(BOOL)settings {
    [self ensureWebServer:^(BOOL ready) {
        if (!ready) {
            [self showError:[NSError errorWithDomain:@"GQYMenuBar"
                                                code:2
                                            userInfo:@{
                                                NSLocalizedDescriptionKey:
                                                    @"面板服务启动超时，请稍后重试。"
                                            }]];
            return;
        }
        dispatch_async(dispatch_get_main_queue(), ^{
            NSString *urlString = self.panelURL.absoluteString;
            if (settings) {
                urlString = [urlString stringByAppendingString:@"?open=settings"];
            }
            [NSWorkspace.sharedWorkspace openURL:[NSURL URLWithString:urlString]];
        });
    }];
}

// 确保 gqy web 已启动：轮询 /api/health 直到就绪（替代写死的 800ms 延迟）
- (void)ensureWebServer:(void (^)(BOOL ready))completion {
    if (!self.webTask.isRunning) {
        NSError *error = nil;
        NSTask *task = [self assistantTaskWithArguments:@[@"web", @"--no-open"]
                                                  error:&error];
        if (!task || ![task launchAndReturnError:&error]) {
            [self showError:error];
            completion(NO);
            return;
        }
        self.webTask = task;
    }
    [self pollHealthAttempts:20 completion:completion];
}

// ─────────────────────────── 本地推理（llama.cpp）───────────────────────────
// 生命周期跟随菜单栏：启动时检查 127.0.0.1:8080，无服务则自己拉起；退出时杀掉
// 自己拉起的进程。只有「开机自启」被用户启用时，才注册 LaunchAgent 随登录自启。

- (void)ensureLlamaServer {
    // 已有服务（外部/LaunchAgent 拉起）→ 复用，不重复 spawn
    NSMutableURLRequest *request = [NSMutableURLRequest
        requestWithURL:[NSURL URLWithString:@"http://127.0.0.1:8080/v1/models"]];
    request.timeoutInterval = 1;
    NSURLSessionDataTask *probe = [NSURLSession.sharedSession
        dataTaskWithRequest:request
          completionHandler:^(NSData *data, NSURLResponse *response, NSError *error) {
        NSHTTPURLResponse *http = (NSHTTPURLResponse *)response;
        if (error || http.statusCode != 200) {
            [self spawnLlamaServer];
        }
    }];
    [probe resume];
}

- (void)spawnLlamaServer {
    if (self.llamaTask.isRunning) {
        return;
    }
    NSString *binary = @"/opt/homebrew/bin/llama-server";
    if (![NSFileManager.defaultManager fileExistsAtPath:binary]) {
        return;
    }
    NSTask *task = [[NSTask alloc] init];
    task.executableURL = [NSURL fileURLWithPath:binary];
    task.arguments = @[
        @"-m", [self llamaModelPath],
        @"--host", @"127.0.0.1",
        @"--port", @"8080",
        @"-c", @"8192",
        @"--alias", @"qwen3-abl-nothink",
        @"--chat-template-file", [self llamaTemplatePath],
    ];
    [task launch];
    self.llamaTask = task;
}

- (NSString *)llamaModelPath {
    NSString *home = NSHomeDirectory();
    return [home stringByAppendingPathComponent:@"llama-models/qwen3-abliterated-8b-q4.gguf"];
}

- (NSString *)llamaTemplatePath {
    NSString *home = NSHomeDirectory();
    return [home stringByAppendingPathComponent:@"llama-models/qwen3-nothink.jinja"];
}

- (void)pollHealthAttempts:(int)remaining completion:(void (^)(BOOL ready))completion {
    if (remaining <= 0) {
        completion(NO);
        return;
    }
    NSMutableURLRequest *request = [NSMutableURLRequest
        requestWithURL:[NSURL URLWithString:@"http://127.0.0.1:4096/api/health"]];
    request.timeoutInterval = 1;
    NSURLSessionDataTask *task = [NSURLSession.sharedSession
        dataTaskWithRequest:request
          completionHandler:^(NSData *data, NSURLResponse *response, NSError *error) {
        NSHTTPURLResponse *http = (NSHTTPURLResponse *)response;
        dispatch_async(dispatch_get_main_queue(), ^{
            if (!error && http.statusCode == 200 && data.length > 0) {
                completion(YES);
            } else {
                dispatch_after(dispatch_time(DISPATCH_TIME_NOW, 500 * NSEC_PER_MSEC),
                               dispatch_get_main_queue(), ^{
                    [self pollHealthAttempts:remaining - 1 completion:completion];
                });
            }
        });
    }];
    [task resume];
}

// ─────────────────────────── 备份 ───────────────────────────

- (void)backupNow:(id)sender {
    (void)sender;
    if (self.backupTask.isRunning) {
        return;
    }
    NSError *error = nil;
    NSTask *task = [self assistantTaskWithArguments:@[@"backup", @"now"]
                                              error:&error];
    if (!task) {
        [self showError:error];
        return;
    }
    task.standardOutput = [NSPipe pipe];
    task.standardError = [NSPipe pipe];
    self.backupItem.title = @"正在备份…";
    self.backupItem.enabled = NO;
    [self setStatusIconBackup:YES];
    __weak typeof(self) weakSelf = self;
    task.terminationHandler = ^(NSTask *finished) {
        dispatch_async(dispatch_get_main_queue(), ^{
            weakSelf.backupItem.title = finished.terminationStatus == 0
                ? @"备份完成"
                : @"备份失败（点此重试）";
            weakSelf.backupItem.enabled = YES;
            weakSelf.backupTask = nil;
            [weakSelf setStatusIconBackup:NO];
            [weakSelf refreshStatus];
        });
    };
    if (![task launchAndReturnError:&error]) {
        self.backupItem.title = @"备份失败（点此重试）";
        self.backupItem.enabled = YES;
        [self setStatusIconBackup:NO];
        [self showError:error];
        return;
    }
    self.backupTask = task;
}

// 状态栏图标随状态变化：空闲恢复顾清影头像，备份中 clock 旋转动画（用户可直接看到备份在跑）
- (void)setStatusIconBackup:(BOOL)backup {
    self.backupInProgress = backup;
    NSString *symbol = backup ? @"externaldrive.fill.badge.clock" : nil;
    self.statusItem.button.image = backup
        ? [NSImage imageWithSystemSymbolName:symbol accessibilityDescription:@"顾清影"]
        : self.statusItemIcon;
    self.statusItem.button.toolTip = backup ? @"顾清影 —— 正在备份…" : @"顾清影 —— 点开菜单";
    CALayer *layer = self.statusItem.button.layer;
    if (backup) {
        [layer removeAnimationForKey:@"gqyBackupSpin"];
        CABasicAnimation *spin = [CABasicAnimation animationWithKeyPath:@"transform.rotation"];
        spin.fromValue = @(0);
        spin.toValue = @(2 * M_PI);
        spin.duration = 1.2;
        spin.repeatCount = HUGE_VALF;
        [layer addAnimation:spin forKey:@"gqyBackupSpin"];
    } else {
        [layer removeAnimationForKey:@"gqyBackupSpin"];
    }
}

// ─────────────────────────── 状态区（异步刷新） ───────────────────────────

- (void)refreshStatus {
    dispatch_async(dispatch_get_global_queue(QOS_CLASS_USER_INITIATED, 0), ^{
        NSString *model = [self readModelStatus];
        NSString *memory = [self readMemoryStatus];
        NSString *backup = [self readBackupStatus];
        dispatch_async(dispatch_get_main_queue(), ^{
            if (model.length > 0) {
                BOOL modelReady = [model containsString:@"未配置"] == NO;
                [self setStatusItem:self.statusModelItem
                              title:model
                              color:modelReady ? [NSColor systemCyanColor] : [NSColor systemGrayColor]];
            }
            if (memory.length > 0) {
                BOOL memoryReady = [memory containsString:@"不可用"] == NO;
                [self setStatusItem:self.statusMemoryItem
                              title:memory
                              color:memoryReady ? [NSColor systemPurpleColor] : [NSColor systemGrayColor]];
            }
            if (backup.length > 0) {
                BOOL backupOK = [backup containsString:@"未设置"] == NO;
                [self setStatusItem:self.statusBackupItem
                              title:backup
                              color:backupOK ? [NSColor systemGreenColor] : [NSColor systemGrayColor]];
            }
        });
    });
}

// 从 config.jsonc（JSONC，容忍注释）抠「模型：provider / model」
- (NSString *)readModelStatus {
    NSString *configPath = [self.assistantHome URLByAppendingPathComponent:@"config.jsonc"].path;
    NSString *text = [NSString stringWithContentsOfFile:configPath
                                               encoding:NSUTF8StringEncoding
                                                  error:nil];
    if (text.length == 0) {
        return @"模型：未配置";
    }
    // active_provider_models[0]: { "provider_id": "x", "model": "y" }
    NSRegularExpression *poolRegex = [NSRegularExpression
        regularExpressionWithPattern:@"\"active_provider_models\"\\s*:\\s*\\[\\s*\\{\\s*\"provider_id\"\\s*:\\s*\"([^\"]+)\"\\s*,\\s*\"model\"\\s*:\\s*\"([^\"]+)\""
                             options:0
                               error:nil];
    NSTextCheckingResult *poolMatch = [poolRegex firstMatchInString:text
                                                           options:0
                                                             range:NSMakeRange(0, text.length)];
    if (poolMatch && [poolMatch rangeAtIndex:1].location != NSNotFound) {
        NSString *provider = [text substringWithRange:[poolMatch rangeAtIndex:1]];
        NSString *model = [text substringWithRange:[poolMatch rangeAtIndex:2]];
        return [NSString stringWithFormat:@"模型：%@ / %@", provider, model];
    }
    NSRegularExpression *providerRegex = [NSRegularExpression
        regularExpressionWithPattern:@"\"active_provider\"\\s*:\\s*\"([^\"]+)\""
                             options:0
                               error:nil];
    NSTextCheckingResult *providerMatch = [providerRegex firstMatchInString:text
                                                                   options:0
                                                                     range:NSMakeRange(0, text.length)];
    if (providerMatch && [providerMatch rangeAtIndex:1].location != NSNotFound) {
        return [NSString stringWithFormat:@"模型：%@",
            [text substringWithRange:[providerMatch rangeAtIndex:1]]];
    }
    return @"模型：未配置";
}

// 记忆条数：跑 gqy memory stats 取 episodes
- (NSString *)readMemoryStatus {
    NSError *error = nil;
    NSTask *task = [self assistantTaskWithArguments:@[@"memory", @"stats"] error:&error];
    if (!task) {
        return nil;
    }
    NSPipe *pipe = [NSPipe pipe];
    task.standardOutput = pipe;
    if (![task launchAndReturnError:&error]) {
        return nil;
    }
    [task waitUntilExit];
    if (task.terminationStatus != 0) {
        return nil;
    }
    NSData *data = [pipe.fileHandleForReading readDataToEndOfFile];
    NSDictionary *json = [NSJSONSerialization JSONObjectWithData:data options:0 error:nil];
    NSNumber *episodes = json[@"episodes"];
    if (![episodes isKindOfClass:NSNumber.class]) {
        return nil;
    }
    return [NSString stringWithFormat:@"记忆：%@ 条日记", episodes];
}

// 上次备份时间：读 backup/repository 的最近 commit
- (NSString *)readBackupStatus {
    NSURL *repo = [self.assistantHome URLByAppendingPathComponent:@"backup/repository"
                                                      isDirectory:YES];
    NSTask *task = [[NSTask alloc] init];
    task.executableURL = [NSURL fileURLWithPath:@"/usr/bin/git"];
    task.arguments = @[@"-C", repo.path, @"log", @"-1", @"--format=%ct"];
    NSPipe *pipe = [NSPipe pipe];
    task.standardOutput = pipe;
    task.standardError = [NSPipe pipe];
    if (![task launchAndReturnError:nil]) {
        return nil;
    }
    [task waitUntilExit];
    if (task.terminationStatus != 0) {
        return nil;
    }
    NSData *data = [pipe.fileHandleForReading readDataToEndOfFile];
    NSString *timestamp = [[NSString alloc] initWithData:data encoding:NSUTF8StringEncoding];
    timestamp = [timestamp stringByTrimmingCharactersInSet:NSCharacterSet.whitespaceAndNewlineCharacterSet];
    if (timestamp.length == 0) {
        return @"备份：还没有快照";
    }
    NSTimeInterval last = timestamp.doubleValue;
    NSTimeInterval now = NSDate.date.timeIntervalSince1970;
    NSInteger seconds = (NSInteger)(now - last);
    NSString *relative;
    if (seconds < 60) {
        relative = @"刚刚";
    } else if (seconds < 3600) {
        relative = [NSString stringWithFormat:@"%ld 分钟前", seconds / 60];
    } else if (seconds < 86400) {
        relative = [NSString stringWithFormat:@"%ld 小时前", seconds / 3600];
    } else {
        relative = [NSString stringWithFormat:@"%ld 天前", seconds / 86400];
    }
    return [NSString stringWithFormat:@"备份：%@", relative];
}

// ─────────────────────────── 其他 ───────────────────────────

- (void)openAssistantHome:(id)sender {
    (void)sender;
    NSError *error = nil;
    if (![NSFileManager.defaultManager createDirectoryAtURL:self.assistantHome
                                withIntermediateDirectories:YES
                                                 attributes:nil
                                                      error:&error]) {
        [self showError:error];
        return;
    }
    [NSWorkspace.sharedWorkspace openURL:self.assistantHome];
}

- (void)openConfigFile:(id)sender {
    (void)sender;
    NSURL *config = [self.assistantHome URLByAppendingPathComponent:@"config/config.jsonc"];
    if (![NSFileManager.defaultManager fileExistsAtPath:config.path]) {
        config = [self.assistantHome URLByAppendingPathComponent:@"config.jsonc"];
    }
    if (![NSFileManager.defaultManager fileExistsAtPath:config.path]) {
        [self showError:[NSError errorWithDomain:@"GQYMenuBar"
                                            code:3
                                        userInfo:@{
                                            NSLocalizedDescriptionKey: @"配置文件不存在。"
                                        }]];
        return;
    }
    [NSWorkspace.sharedWorkspace openURL:config];
}

- (void)quit:(id)sender {
    (void)sender;
    [self shutdownDaemon];
    [NSApp terminate:nil];
}

// 优雅关闭后台守护进程（gqy web）：POST /api/shutdown →
// 停 serve → agent 结束（pi 进程组随 agent drop 被清理）→ 进程退出。
// 无论 daemon 是本 App 拉起还是终端手动启动，都会统一收尾。
- (void)shutdownDaemon {
    NSMutableURLRequest *request = [NSMutableURLRequest
        requestWithURL:[NSURL URLWithString:@"http://127.0.0.1:4096/api/shutdown"]];
    request.HTTPMethod = @"POST";
    request.timeoutInterval = 3;
    // 同步发送：确保退出前请求已发出
    __unused NSData *data = [NSURLConnection sendSynchronousRequest:request
                                                  returningResponse:nil
                                                              error:nil];
    // 短暂等待 daemon 收尾（pi 进程组清理），再让自己退出
    usleep(300 * 1000);
}

- (NSURL *)loginAgentPlist {
    NSURL *launchAgents = [[NSFileManager.defaultManager
        URLForDirectory:NSLibraryDirectory
               inDomain:NSUserDomainMask
      appropriateForURL:nil
                 create:YES
                  error:nil]
        URLByAppendingPathComponent:@"LaunchAgents" isDirectory:YES];
    return [launchAgents URLByAppendingPathComponent:@"dev.gqy.menubar.plist"];
}

- (BOOL)loginItemEnabled {
    return [NSFileManager.defaultManager
        fileExistsAtPath:self.loginAgentPlist.path];
}

- (void)refreshLoginItemState {
    self.loginItemMenu.state =
        self.loginItemEnabled ? NSControlStateValueOn : NSControlStateValueOff;
}

- (void)menuWillOpen:(NSMenu *)menu {
    (void)menu;
    [self refreshLoginItemState];
    // 每次打开菜单都刷新状态区：WebUI/CLI 改过配置或备份后菜单栏即时同步
    [self refreshStatus];
}

- (void)toggleLoginItem:(id)sender {
    (void)sender;
    if (self.loginItemEnabled) {
        [self removeLoginItem];
    } else {
        [self installLoginItem];
    }
}

- (void)installLoginItem {
    NSURL *plist = self.loginAgentPlist;
    NSError *error = nil;
    if (![NSFileManager.defaultManager
            createDirectoryAtURL:plist.URLByDeletingLastPathComponent
        withIntermediateDirectories:YES
                         attributes:nil
                              error:&error]) {
        [self showError:error];
        return;
    }
    NSDictionary *configuration = @{
        @"Label": @"dev.gqy.menubar",
        @"ProgramArguments": @[@"/usr/bin/open", NSBundle.mainBundle.bundleURL.path],
        @"RunAtLoad": @YES,
        @"ProcessType": @"Interactive",
        @"EnvironmentVariables": @{
            @"GQY_HOME": self.assistantHome.path,
        },
    };
    NSData *data = [NSPropertyListSerialization
        dataWithPropertyList:configuration
                      format:NSPropertyListXMLFormat_v1_0
                     options:0
                       error:&error];
    if (!data ||
        ![data writeToURL:plist options:NSDataWritingAtomic error:&error]) {
        [self showError:error];
        return;
    }
    [self installLlamaLaunchAgent];
    [self refreshLoginItemState];
    [self showInfo:@"已开启开机自启"
               detail:@"顾清影与本地推理（llama.cpp）将在下次登录时自动启动。"];
}

// 开机自启启用时，同步注册 llama.cpp 的 LaunchAgent（随登录一起拉起本地推理）
- (void)installLlamaLaunchAgent {
    NSString *home = NSHomeDirectory();
    NSString *plistPath = [home
        stringByAppendingPathComponent:@"Library/LaunchAgents/com.gqy.llamacpp.plist"];
    NSDictionary *configuration = @{
        @"Label": @"com.gqy.llamacpp",
        @"ProgramArguments": @[
            @"/opt/homebrew/bin/llama-server",
            @"-m", [self llamaModelPath],
            @"--host", @"127.0.0.1",
            @"--port", @"8080",
            @"-c", @"8192",
            @"--alias", @"qwen3-abl-nothink",
            @"--chat-template-file", [self llamaTemplatePath],
        ],
        @"RunAtLoad": @YES,
        @"KeepAlive": @YES,
    };
    NSData *data = [NSPropertyListSerialization
        dataWithPropertyList:configuration
                      format:NSPropertyListXMLFormat_v1_0
                     options:0
                       error:nil];
    if (data) {
        [data writeToFile:plistPath options:NSDataWritingAtomic error:nil];
        [self runLaunchCtl:@[@"bootout", [self launchctlTarget], @"com.gqy.llamacpp"]];
        [self runLaunchCtl:@[@"load", plistPath]];
    }
}

- (void)removeLoginItem {
    [self runLaunchCtl:@[@"bootout", [self launchctlTarget], @"dev.gqy.menubar"]];
    // 关闭自启时同步移除 llama.cpp 随登录自启（当前若在跑，由本 App 进程继续托管）
    [self runLaunchCtl:@[@"bootout", [self launchctlTarget], @"com.gqy.llamacpp"]];
    NSString *home = NSHomeDirectory();
    NSString *llamaPlist = [home
        stringByAppendingPathComponent:@"Library/LaunchAgents/com.gqy.llamacpp.plist"];
    [NSFileManager.defaultManager removeItemAtPath:llamaPlist error:nil];
    NSError *error = nil;
    [NSFileManager.defaultManager removeItemAtURL:self.loginAgentPlist
                                            error:&error];
    [self refreshLoginItemState];
    [self showInfo:@"已关闭开机自启" detail:@"下次登录将不再自动启动。"];
}

- (NSString *)launchctlTarget {
    return [NSString stringWithFormat:@"gui/%d", (int)getuid()];
}

- (void)runLaunchCtl:(NSArray<NSString *> *)arguments {
    NSTask *task = [[NSTask alloc] init];
    task.executableURL = [NSURL fileURLWithPath:@"/bin/launchctl"];
    task.arguments = arguments;
    [task launch];
    [task waitUntilExit];
}

- (NSString *)shellQuote:(NSString *)value {
    return [NSString stringWithFormat:@"'%@'",
        [value stringByReplacingOccurrencesOfString:@"'" withString:@"'\\''"]];
}

- (void)showInfo:(NSString *)title detail:(NSString *)detail {
    NSAlert *alert = [[NSAlert alloc] init];
    alert.messageText = title;
    alert.informativeText = detail;
    [alert addButtonWithTitle:@"知道了"];
    [alert runModal];
}

- (void)showError:(NSError *)error {
    NSAlert *alert = [[NSAlert alloc] init];
    alert.alertStyle = NSAlertStyleWarning;
    alert.messageText = @"顾清影暂时无法完成这个操作";
    alert.informativeText = error.localizedDescription ?: @"未知错误";
    [alert addButtonWithTitle:@"知道了"];
    [alert runModal];
}

@end

int main(int argc, const char *argv[]) {
    (void)argc;
    (void)argv;
    @autoreleasepool {
        NSApplication *application = NSApplication.sharedApplication;
        GQYMenuBarDelegate *delegate = [[GQYMenuBarDelegate alloc] init];
        application.delegate = delegate;
        [application run];
    }
    return 0;
}
