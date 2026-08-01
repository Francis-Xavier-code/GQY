#import <AppKit/AppKit.h>
#import <QuartzCore/QuartzCore.h>
#import <WebKit/WebKit.h>
#import <Carbon/Carbon.h>
#import <unistd.h>

/**
 * 顾清影 菜单栏 App
 * - 左键点击状态栏图标弹出菜单（保持习惯）
 * - 「面板」是独立 App 窗口（NSPanel + WKWebView），可拖动缩放，不依赖浏览器
 * - 状态栏图标随状态变化（空闲 sparkles / 备份中 clock）
 * - 菜单含状态区（模型/记忆/备份时间，异步刷新）+ 常用功能
 */
@interface GQYMenuBarDelegate : NSObject <NSApplicationDelegate, NSMenuDelegate, NSWindowDelegate, WKScriptMessageHandler>
@property(nonatomic, strong) NSStatusItem *statusItem;
@property(nonatomic, strong) NSTask *webTask;
@property(nonatomic, strong) NSTask *backupTask;
@property(nonatomic, strong) NSMenuItem *backupItem;
@property(nonatomic, strong) NSMenuItem *loginItemMenu;
@property(nonatomic, strong) NSMenuItem *statusModelItem;
@property(nonatomic, strong) NSMenuItem *statusMemoryItem;
@property(nonatomic, strong) NSMenuItem *statusBackupItem;
@property(nonatomic, strong) NSWindow *panelWindow;
@property(nonatomic, strong) WKWebView *webView;
@property(nonatomic, strong) NSPanel *miniWindow;
@property(nonatomic, strong) WKWebView *miniWebView;
@property(nonatomic, assign) BOOL backupInProgress;
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
        self.statusItem.button.image = [NSImage
            imageWithSystemSymbolName:@"sparkles"
            accessibilityDescription:@"顾清影"];
    }
    self.statusItem.button.toolTip = @"顾清影 —— 点开菜单，面板在 App 内（⌥G 快速打开迷你对话）";

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
    NSMenuItem *miniItem = [self itemWithTitle:@"迷你对话 ⌥G"
                                      symbol:@"text.bubble"
                                      action:@selector(toggleMiniWindow:)];
    [menu addItem:miniItem];
    NSMenuItem *panelItem = [self itemWithTitle:@"打开面板"
                                         symbol:@"square.grid.2x2"
                                         action:@selector(openWebPanel:)];
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
    [menu addItem:[self itemWithTitle:@"打开独立主目录"
                               symbol:@"folder"
                               action:@selector(openAssistantHome:)]];
    [menu addItem:[self itemWithTitle:@"打开配置文件"
                               symbol:@"doc.text"
                               action:@selector(openConfigFile:)]];
    [menu addItem:[NSMenuItem separatorItem]];
    self.loginItemMenu = [self itemWithTitle:@"开机自启"
                                      symbol:@"power"
                                      action:@selector(toggleLoginItem:)];
    [menu addItem:self.loginItemMenu];
    [menu addItem:[NSMenuItem separatorItem]];
    [menu addItem:[self itemWithTitle:@"退出顾清影"
                               symbol:@"xmark.circle"
                               action:@selector(quit:)]];
    self.statusItem.menu = menu;
    menu.delegate = self;
    [self refreshLoginItemState];
    [self refreshStatus];
    [self registerGlobalHotkey];

    // 调试/自测：`GQYMenuBar --mini` 启动即开迷你对话窗口
    NSArray *arguments = NSProcessInfo.processInfo.arguments;
    if ([arguments containsObject:@"--mini"]) {
        dispatch_after(dispatch_time(DISPATCH_TIME_NOW, (int64_t)(1.0 * NSEC_PER_SEC)),
                       dispatch_get_main_queue(), ^{
            [self toggleMiniWindow:nil];
        });
    }
}

// ⌥G 全局快捷键：任何应用里按下都弹出迷你对话（面板窗口）
static EventHotKeyRef g_hotkey_ref = NULL;
static OSStatus gqy_hotkey_handler(EventHandlerCallRef nextHandler,
                                   EventRef event,
                                   void *userData) {
    (void)nextHandler;
    (void)event;
    GQYMenuBarDelegate *delegate = (__bridge GQYMenuBarDelegate *)userData;
    [delegate toggleMiniWindow:nil];
    return noErr;
}

- (void)registerGlobalHotkey {
    EventHotKeyID hotkey_id = { .signature = 'GQYH', .id = 1 };
    EventTypeSpec event_type = { .eventClass = kEventClassKeyboard,
                                 .eventKind = kEventHotKeyPressed };
    InstallEventHandler(GetEventDispatcherTarget(),
                        gqy_hotkey_handler,
                        1,
                        &event_type,
                        (__bridge void *)self,
                        NULL);
    // ⌥G：Option + G（避开中文输入法的 ⌥Space）
    RegisterEventHotKey(kVK_ANSI_G, optionKey, hotkey_id,
                        GetEventDispatcherTarget(), 0, &g_hotkey_ref);
}

- (void)restartWebServer:(id)sender {
    (void)sender;
    if (self.webTask.isRunning) {
        [self.webTask terminate];
    }
    self.webTask = nil;
    [self showInfo:@"面板服务已重启" detail:@"下次打开面板时自动重新启动，并加载最新配置。"];
}

- (void)applicationWillTerminate:(NSNotification *)notification {
    (void)notification;
    if (self.webTask.isRunning) {
        [self.webTask terminate];
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

// ─────────────────────────── 独立窗口面板（WKWebView） ───────────────────────────

- (NSURL *)panelURL {
    return [NSURL URLWithString:@"http://127.0.0.1:4096"];
}

- (void)openWebPanel:(id)sender {
    (void)sender;
    [self openPanelWithSettings:NO];
}

// 打开面板并直接展开配置抽屉（等价于终端里的 gqy config，GUI 版）
- (void)openConfigPanel:(id)sender {
    (void)sender;
    [self openPanelWithSettings:YES];
}

- (void)openPanelWithSettings:(BOOL)settings {
    if (self.panelWindow.isVisible) {
        [self.panelWindow makeKeyAndOrderFront:nil];
        [NSApp activateIgnoringOtherApps:YES];
        if (settings) {
            [self.webView evaluateJavaScript:@"window.__gqyOpenSettings && window.__gqyOpenSettings()" completionHandler:nil];
        }
        return;
    }
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
        [self showPanelWithSettings:settings];
    }];
}

// 面板是独立 App 窗口：可拖动、可缩放、独立于状态栏存在；
// 打开时切换到 Regular 激活策略，Dock 出现图标；关闭时切回 Accessory
- (void)showPanelWithSettings:(BOOL)settings {
    if (!self.panelWindow) {
        NSRect frame = NSMakeRect(0, 0, 720, 680);
        self.panelWindow = [[NSPanel alloc]
            initWithContentRect:frame
                      styleMask:(NSWindowStyleMaskTitled |
                                 NSWindowStyleMaskClosable |
                                 NSWindowStyleMaskResizable |
                                 NSWindowStyleMaskFullSizeContentView)
                        backing:NSBackingStoreBuffered
                          defer:NO];
        self.panelWindow.title = @"顾清影 · 面板";
        self.panelWindow.minSize = NSMakeSize(560, 480);
        self.panelWindow.delegate = self;
        self.panelWindow.releasedWhenClosed = NO;

        WKWebView *webView = [[WKWebView alloc] initWithFrame:self.panelWindow.contentView.bounds];
        webView.autoresizingMask = NSViewWidthSizable | NSViewHeightSizable;
        webView.allowsMagnification = YES;
        self.webView = webView;
        self.panelWindow.contentView = webView;
        [self.panelWindow center];
    }
    [self.panelWindow makeKeyAndOrderFront:nil];
    [NSApp setActivationPolicy:NSApplicationActivationPolicyRegular];
    [NSApp activateIgnoringOtherApps:YES];
    NSString *urlString = self.panelURL.absoluteString;
    if (settings) {
        urlString = [urlString stringByAppendingString:@"?open=settings"];
    }
    if (![self.webView.URL.absoluteString hasPrefix:urlString]) {
        [self.webView loadRequest:[NSURLRequest requestWithURL:[NSURL URLWithString:urlString]]];
    } else {
        [self.webView reload];
    }
}

// ─────────────────────────── 迷你对话窗口（Gemini 式） ───────────────────────────
// 小圆角窗口内嵌同源 WebView（?mini=1）：隐藏侧栏/顶栏，只留对话区+输入框。
// 输入后走 WebUI 自己的 SSE 流，回答、头像思考动画全部复用。
// 放大按钮（⤢）→ WKScriptMessageHandler 收到 gqyExpand → 切换成完整面板。

- (void)toggleMiniWindow:(id)sender {
    (void)sender;
    if (self.miniWindow.isVisible) {
        [self.miniWindow orderOut:nil];
        [NSApp setActivationPolicy:NSApplicationActivationPolicyAccessory];
        return;
    }
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
        [self showMiniWindow];
    }];
}

- (void)showMiniWindow {
    if (!self.miniWindow) {
        NSRect frame = NSMakeRect(0, 0, 480, 340);
        self.miniWindow = [[NSPanel alloc]
            initWithContentRect:frame
                      styleMask:(NSWindowStyleMaskNonactivatingPanel |
                                 NSWindowStyleMaskFullSizeContentView)
                        backing:NSBackingStoreBuffered
                          defer:NO];
        self.miniWindow.title = @"顾清影 · 迷你对话";
        self.miniWindow.level = NSFloatingWindowLevel;
        self.miniWindow.hidesOnDeactivate = NO;
        self.miniWindow.releasedWhenClosed = NO;
        self.miniWindow.delegate = self;
        // 圆角 + 无标题栏
        self.miniWindow.titleVisibility = NSWindowTitleHidden;
        self.miniWindow.titlebarAppearsTransparent = YES;
        self.miniWindow.backgroundColor = [NSColor clearColor];
        [self.miniWindow setMovableByWindowBackground:YES];

        // 内容容器：圆角裁剪
        NSView *container = [[NSView alloc] initWithFrame:self.miniWindow.contentView.bounds];
        container.autoresizingMask = NSViewWidthSizable | NSViewHeightSizable;
        container.wantsLayer = YES;
        container.layer.cornerRadius = 20;
        container.layer.masksToBounds = YES;
        self.miniWindow.contentView = container;

        WKWebViewConfiguration *config = [[WKWebViewConfiguration alloc] init];
        [config.userContentController addScriptMessageHandler:self name:@"gqyExpand"];

        WKWebView *webView = [[WKWebView alloc] initWithFrame:container.bounds
                                                configuration:config];
        webView.autoresizingMask = NSViewWidthSizable | NSViewHeightSizable;
        webView.allowsMagnification = NO;
        self.miniWebView = webView;
        [container addSubview:webView];

        // 默认位置：Dock 上方（visibleFrame 已排除 Dock 区域），
        // 屏幕底部中央偏右，贴近 Dock 但浮在其上
        NSRect screen = [NSScreen mainScreen].visibleFrame;
        CGFloat x = NSMidX(screen) - frame.size.width / 2 + 80;
        CGFloat y = NSMinY(screen) + 12;
        [self.miniWindow setFrameOrigin:NSMakePoint(x, y)];
    }
    NSString *urlString = [NSString stringWithFormat:@"%@?mini=1", self.panelURL.absoluteString];
    if (![self.miniWebView.URL.absoluteString hasPrefix:urlString]) {
        [self.miniWebView loadRequest:[NSURLRequest requestWithURL:[NSURL URLWithString:urlString]]];
    } else {
        [self.miniWebView reload];
    }
    [self.miniWindow makeKeyAndOrderFront:nil];
    [NSApp activateIgnoringOtherApps:YES];
}

// 迷你窗口放大按钮 → 切换成完整面板（关迷你，开面板）
- (void)userContentController:(WKUserContentController *)userContentController
      didReceiveScriptMessage:(WKScriptMessage *)message {
    (void)userContentController;
    if ([message.name isEqualToString:@"gqyExpand"]) {
        [self.miniWindow orderOut:nil];
        [self openWebPanel:nil];
    }
}

- (void)windowWillClose:(NSNotification *)notification {
    if (notification.object == self.panelWindow ||
        notification.object == self.miniWindow) {
        // 关窗口不杀 web 服务（下次秒开），同时 Dock 图标收回
        [NSApp setActivationPolicy:NSApplicationActivationPolicyAccessory];
    }
}

- (BOOL)windowShouldClose:(NSWindow *)sender {
    (void)sender;
    return YES;
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

// 状态栏图标随状态变化：空闲 sparkles，备份中 clock 旋转动画（用户可直接看到备份在跑）
- (void)setStatusIconBackup:(BOOL)backup {
    self.backupInProgress = backup;
    NSString *symbol = backup ? @"externaldrive.fill.badge.clock" : @"sparkles";
    self.statusItem.button.image = [NSImage
        imageWithSystemSymbolName:symbol
        accessibilityDescription:@"顾清影"];
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
    [NSApp terminate:nil];
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
    [self refreshLoginItemState];
    [self showInfo:@"已开启开机自启"
               detail:@"顾清影将在下次登录时自动启动。"];
}

- (void)removeLoginItem {
    [self runLaunchCtl:@[@"bootout", [self launchctlTarget], @"dev.gqy.menubar"]];
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
