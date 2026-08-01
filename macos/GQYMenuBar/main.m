#import <AppKit/AppKit.h>

@interface GQYMenuBarDelegate : NSObject <NSApplicationDelegate>
@property(nonatomic, strong) NSStatusItem *statusItem;
@property(nonatomic, strong) NSTask *webTask;
@property(nonatomic, strong) NSTask *backupTask;
@property(nonatomic, strong) NSMenuItem *backupItem;
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
    self.statusItem.button.toolTip = @"顾清影";

    NSMenu *menu = [[NSMenu alloc] init];
    [menu addItem:[self itemWithTitle:@"打开终端对话"
                              action:@selector(openTerminalChat:)]];
    [menu addItem:[self itemWithTitle:@"打开本地面板"
                              action:@selector(openWebPanel:)]];
    [menu addItem:[NSMenuItem separatorItem]];
    self.backupItem = [self itemWithTitle:@"立即备份记忆"
                                   action:@selector(backupNow:)];
    [menu addItem:self.backupItem];
    [menu addItem:[self itemWithTitle:@"打开独立主目录"
                              action:@selector(openAssistantHome:)]];
    [menu addItem:[NSMenuItem separatorItem]];
    [menu addItem:[self itemWithTitle:@"退出顾清影"
                              action:@selector(quit:)]];
    self.statusItem.menu = menu;
}

- (void)applicationWillTerminate:(NSNotification *)notification {
    (void)notification;
    if (self.webTask.isRunning) {
        [self.webTask terminate];
    }
}

- (NSMenuItem *)itemWithTitle:(NSString *)title action:(SEL)action {
    NSMenuItem *item = [[NSMenuItem alloc] initWithTitle:title
                                                 action:action
                                          keyEquivalent:@""];
    item.target = self;
    return item;
}

- (NSURL *)assistantHome {
    NSString *configured = NSProcessInfo.processInfo.environment[@"GQY_HOME"];
    if (configured.length > 0) {
        return [NSURL fileURLWithPath:configured isDirectory:YES].standardizedURL;
    }
    return [[NSFileManager.defaultManager URLsForDirectory:NSApplicationSupportDirectory
                                                 inDomains:NSUserDomainMask].firstObject
        URLByAppendingPathComponent:@"GQY"
                        isDirectory:YES];
}

- (NSURL *)assistantBinary:(NSError **)error {
    NSDictionary<NSString *, NSString *> *environment =
        NSProcessInfo.processInfo.environment;
    NSString *workingDirectory = environment[@"PWD"];
    NSMutableArray<NSString *> *candidates = [NSMutableArray array];
    NSString *bundled = [NSBundle.mainBundle pathForResource:@"miyu" ofType:nil];
    if (bundled.length > 0) {
        [candidates addObject:bundled];
    }
    if (environment[@"GQY_BIN"].length > 0) {
        [candidates addObject:environment[@"GQY_BIN"]];
    }
    [candidates addObjectsFromArray:@[
        @"/opt/homebrew/bin/miyu",
        @"/usr/local/bin/miyu",
    ]];
    if (workingDirectory.length > 0) {
        [candidates addObject:[workingDirectory
                                  stringByAppendingPathComponent:@"target/release/miyu"]];
        [candidates addObject:[workingDirectory
                                  stringByAppendingPathComponent:@"target/debug/miyu"]];
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
                                         @"找不到 miyu 后端。请设置 GQY_BIN 为编译后的可执行文件绝对路径。"
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

- (void)openWebPanel:(id)sender {
    (void)sender;
    NSError *error = nil;
    if (!self.webTask.isRunning) {
        NSTask *task = [self assistantTaskWithArguments:@[@"web", @"--no-open"]
                                                  error:&error];
        if (!task || ![task launchAndReturnError:&error]) {
            [self showError:error];
            return;
        }
        self.webTask = task;
    }
    dispatch_after(dispatch_time(DISPATCH_TIME_NOW, 800 * NSEC_PER_MSEC),
                   dispatch_get_main_queue(), ^{
        [NSWorkspace.sharedWorkspace
            openURL:[NSURL URLWithString:@"http://127.0.0.1:4096"]];
    });
}

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

- (NSString *)shellQuote:(NSString *)value {
    return [NSString stringWithFormat:@"'%@'",
        [value stringByReplacingOccurrencesOfString:@"'" withString:@"'\\''"]];
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
