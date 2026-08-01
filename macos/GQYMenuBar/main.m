#import <AppKit/AppKit.h>
#import <WebKit/WebKit.h>
#import <unistd.h>

/**
 * 顾清影 菜单栏 App
 * - 左键点击状态栏图标弹出菜单（保持习惯）
 * - 「面板」在 App 内置 WKWebView 中打开（NSPopover），不再唤起浏览器
 * - 菜单顶部有状态区：模型 / 记忆条数 / 上次备份时间（异步刷新，不卡菜单）
 */
@interface GQYMenuBarDelegate : NSObject <NSApplicationDelegate, NSMenuDelegate>
@property(nonatomic, strong) NSStatusItem *statusItem;
@property(nonatomic, strong) NSTask *webTask;
@property(nonatomic, strong) NSTask *backupTask;
@property(nonatomic, strong) NSMenuItem *backupItem;
@property(nonatomic, strong) NSMenuItem *loginItemMenu;
@property(nonatomic, strong) NSMenuItem *statusModelItem;
@property(nonatomic, strong) NSMenuItem *statusMemoryItem;
@property(nonatomic, strong) NSMenuItem *statusBackupItem;
@property(nonatomic, strong) NSPopover *panelPopover;
@property(nonatomic, strong) WKWebView *webView;
@end

@implementation GQYMenuBarDelegate

- (void)applicationDidFinishLaunching:(NSNotification *)notification {
    (void)notification;
    [NSApp setActivationPolicy:NSApplicationActivationPolicyAccessory];

    self.statusItem = [[NSStatusBar systemStatusBar]
        statusItemWithLength:NSVariableStatusItemLength];
    self.statusItem.button.image = [NSImage
        imageWithSystemSymbolName:@"sparkles"
        accessibilityDescription:@"顾清影"];
    self.statusItem.button.toolTip = @"顾清影 —— 点开菜单，面板在 App 内";

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
    NSMenuItem *panelItem = [self itemWithTitle:@"打开面板"
                                         symbol:@"square.grid.2x2"
                                         action:@selector(openWebPanel:)];
    [menu addItem:panelItem];
    [menu addItem:[self itemWithTitle:@"打开终端对话"
                               symbol:@"terminal"
                               action:@selector(openTerminalChat:)]];
    [menu addItem:[NSMenuItem separatorItem]];
    self.backupItem = [self itemWithTitle:@"立即备份记忆"
                                   symbol:@"externaldrive.fill.badge.checkmark"
                                   action:@selector(backupNow:)];
    [menu addItem:self.backupItem];
    [menu addItem:[self itemWithTitle:@"打开独立主目录"
                               symbol:@"folder"
                               action:@selector(openAssistantHome:)]];
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

- (void)setStatusItem:(NSMenuItem *)item title:(NSString *)title {
    NSMutableParagraphStyle *paragraph = [[NSMutableParagraphStyle alloc] init];
    paragraph.headIndent = 18;
    item.attributedTitle = [[NSAttributedString alloc]
        initWithString:title
        attributes:@{
            NSFontAttributeName: [NSFont systemFontOfSize:11],
            NSForegroundColorAttributeName: NSColor.secondaryLabelColor,
            NSParagraphStyleAttributeName: paragraph,
        }];
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

// ─────────────────────────── 内置面板（WKWebView） ───────────────────────────

- (NSURL *)panelURL {
    return [NSURL URLWithString:@"http://127.0.0.1:4096"];
}

- (void)openWebPanel:(id)sender {
    (void)sender;
    // 先收起菜单，避免与 popover 冲突
    [self.statusItem.menu cancelTracking];
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
        [self showPanel];
    }];
}

- (void)showPanel {
    if (!self.panelPopover) {
        WKWebView *webView = [[WKWebView alloc] initWithFrame:NSMakeRect(0, 0, 430, 640)];
        webView.allowsMagnification = YES;
        webView.allowsBackForwardNavigationGestures = NO;
        NSViewController *controller = [[NSViewController alloc] init];
        controller.view = webView;
        self.webView = webView;
        self.panelPopover = [[NSPopover alloc] init];
        self.panelPopover.contentViewController = controller;
        self.panelPopover.behavior = NSPopoverBehaviorTransient;
    }
    [self.panelPopover showRelativeToRect:self.statusItem.button.bounds
                                   ofView:self.statusItem.button
                            preferredEdge:NSRectEdgeMinY];
    if (![self.webView.URL.absoluteString hasPrefix:self.panelURL.absoluteString]) {
        [self.webView loadRequest:[NSURLRequest requestWithURL:self.panelURL]];
    } else {
        [self.webView reload];
    }
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
    __weak typeof(self) weakSelf = self;
    task.terminationHandler = ^(NSTask *finished) {
        dispatch_async(dispatch_get_main_queue(), ^{
            weakSelf.backupItem.title = finished.terminationStatus == 0
                ? @"备份完成"
                : @"备份失败（点此重试）";
            weakSelf.backupItem.enabled = YES;
            weakSelf.backupTask = nil;
            [weakSelf refreshStatus];
        });
    };
    if (![task launchAndReturnError:&error]) {
        self.backupItem.title = @"备份失败（点此重试）";
        self.backupItem.enabled = YES;
        [self showError:error];
        return;
    }
    self.backupTask = task;
}

// ─────────────────────────── 状态区（异步刷新） ───────────────────────────

- (void)refreshStatus {
    dispatch_async(dispatch_get_global_queue(QOS_CLASS_USER_INITIATED, 0), ^{
        NSString *model = [self readModelStatus];
        NSString *memory = [self readMemoryStatus];
        NSString *backup = [self readBackupStatus];
        dispatch_async(dispatch_get_main_queue(), ^{
            if (model.length > 0) {
                [self setStatusItem:self.statusModelItem title:model];
            }
            if (memory.length > 0) {
                [self setStatusItem:self.statusMemoryItem title:memory];
            }
            if (backup.length > 0) {
                [self setStatusItem:self.statusBackupItem title:backup];
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
