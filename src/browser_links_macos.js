// Native browser tab adapters. No page JavaScript, browser extension or shell interpolation.
ObjC.import('AppKit');
function key(value) {
    var match = /^(https?):\/\/([^/?#]+)(.*)$/i.exec(String(value));
    if (!match) return '';
    var scheme = match[1].toLowerCase();
    var host = match[2].toLowerCase();
    host = host.replace(scheme === 'https' ? /:443$/ : /:80$/, '');
    var path = match[3];
    if (!path || path[0] === '?' || path[0] === '#') path = '/' + path;
    return scheme + '://' + host + path;
}
function run(argv) {
    var url = argv[0];
    var applicationURL = $.NSWorkspace.sharedWorkspace.URLForApplicationToOpenURL($.NSURL.URLWithString(url));
    if (!applicationURL) throw new Error('未找到默认浏览器');
    var bundle = ObjC.unwrap($.NSBundle.bundleWithURL(applicationURL).bundleIdentifier);
    var safari = bundle === 'com.apple.Safari' || bundle === 'com.apple.SafariTechnologyPreview';
    // Detect the native tab protocol from the default browser's scripting dictionary,
    // rather than selecting a browser by a product-name allowlist.
    var nativeBundle = $.NSBundle.bundleWithURL(applicationURL);
    var definition = nativeBundle.objectForInfoDictionaryKey('OSAScriptingDefinition');
    var dictionary = '';
    if (definition) {
        var dictionaryPath = ObjC.unwrap(nativeBundle.resourcePath) + '/' + ObjC.unwrap(definition);
        var contents = $.NSString.stringWithContentsOfFileEncodingError(dictionaryPath, $.NSUTF8StringEncoding, null);
        if (contents) dictionary = ObjC.unwrap(contents);
    }
    var chromium = dictionary.indexOf('name="active tab index"') >= 0 && dictionary.indexOf('name="tab"') >= 0;
    if (!safari && !chromium) throw new Error('默认浏览器未提供兼容的标签复用接口：' + bundle + '；未新建重复标签。');
    var app = Application(bundle);
    if (!app.running()) app.launch();
    var windows = app.windows();
    var nativeApp = $.NSRunningApplication.runningApplicationsWithBundleIdentifier(bundle).firstObject;
    var pid = String(nativeApp.processIdentifier);
    // NSRunningApplication.launchDate can be nil for CLI-launched browsers.
    // Read the kernel start stamp so an old cache cannot reuse a recycled PID/tab id.
    var process = $.NSTask.alloc.init;
    process.launchPath = '/bin/ps';
    process.arguments = ['-p', pid, '-o', 'lstart='];
    process.environment = {LC_ALL: 'C'};
    var pipe = $.NSPipe.pipe;
    process.standardOutput = pipe;
    process.standardError = $.NSPipe.pipe;
    process.launch;
    process.waitUntilExit;
    var bytes = pipe.fileHandleForReading.readDataToEndOfFile;
    var stamp = ObjC.unwrap($.NSString.alloc.initWithDataEncoding(bytes, $.NSUTF8StringEncoding)).trim();
    if (process.terminationStatus !== 0 || !stamp) throw new Error('无法确认默认浏览器进程身份；未创建新标签');
    var token = pid + '/' + stamp;
    var hint = JSON.parse(argv[1] || '{}');
    function remember(win, tab) {
        return JSON.stringify({bundle: bundle, token: token, tab_id: chromium ? tab.id() : null, window_id: win.id()});
    }
    function focus(win, tab, index) {
        if (safari) win.currentTab = tab; else win.activeTabIndex = index + 1;
        if (win.minimized()) win.minimized = false;
        win.index = 1;
        app.activate();
        return remember(win, tab);
    }
    // Chromium tab ids survive reordering, redirects, login flows and DevStart restarts.
    // The browser launch token prevents stale ids from selecting an unrelated new session.
    if (chromium && hint.bundle === bundle && hint.token === token && hint.tab_id !== null) {
        for (var hw = 0; hw < windows.length; hw++) {
            var ht = windows[hw].tabs();
            for (var hi = 0; hi < ht.length; hi++) {
                if (ht[hi].id() === hint.tab_id) {
                    if (key(ht[hi].url()) !== key(url)) ht[hi].url = url;
                    return focus(windows[hw], ht[hi], hi);
                }
            }
        }
    }
    var wanted = key(url);
    for (var w = 0; w < windows.length; w++) {
        var tabs = windows[w].tabs();
        for (var t = 0; t < tabs.length; t++) {
            if (key(tabs[t].url()) === wanted) {
                return focus(windows[w], tabs[t], t);
            }
        }
    }
    if (!windows.length) {
        if (safari) {
            app.Document().make();
            windows = app.windows();
            windows[0].tabs[0].url = url;
        } else {
            app.Window().make();
            windows = app.windows();
            windows[0].tabs[0].url = url;
        }
    } else {
        var tab = app.Tab({url: url});
        windows[0].tabs.push(tab);
        if (safari) windows[0].currentTab = tab;
        else windows[0].activeTabIndex = windows[0].tabs.length;
    }
    var finalTabs = windows[0].tabs();
    return focus(windows[0], finalTabs[finalTabs.length - 1], finalTabs.length - 1);
}
