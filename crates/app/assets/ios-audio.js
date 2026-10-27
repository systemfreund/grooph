// iOS plays Web Audio in the "ambient" session category, which the
// ring/silent switch mutes. <audio> and <video> elements play as "playback"
// and ignore the switch, which is why video sites stay audible. Ask for
// "playback" so the metronome is heard with the switch on silent.
//
// Loaded by the app's index.html and by the MCP App view (mcp/view.js).
(function () {
    // Safari 17+ exposes the session category directly.
    if (navigator.audioSession) {
        try {
            navigator.audioSession.type = "playback";
        } catch (e) {
        }
        return;
    }
    // Older iOS: playing an <audio> element from a user gesture switches the page's session to
    // "playback", and Web Audio follows. Play a looping silent WAV on the first tap or key press.
    var isIos = /iphone|ipad|ipod/i.test(navigator.userAgent) ||
        (navigator.platform === "MacIntel" && navigator.maxTouchPoints > 1);
    if (!isIos) return;

    // A data: URL, which the MCP App host's CSP allows for media (blob: it does not).
    function silentWavUrl() {
        var rate = 8000, samples = 800; // 0.1 s of 8-bit mono silence
        var bytes = new Uint8Array(44 + samples), o = 0;
        function str(s) { for (var i = 0; i < s.length; i++) bytes[o++] = s.charCodeAt(i); }
        function u32(n) { for (var i = 0; i < 4; i++) bytes[o++] = (n >>> (8 * i)) & 255; }
        function u16(n) { bytes[o++] = n & 255; bytes[o++] = (n >>> 8) & 255; }
        str("RIFF"); u32(36 + samples); str("WAVE");
        str("fmt "); u32(16); u16(1); u16(1); u32(rate); u32(rate); u16(1); u16(8);
        str("data"); u32(samples);
        for (var i = 0; i < samples; i++) bytes[o++] = 128; // 8-bit PCM silence is 128
        return "data:audio/wav;base64," + btoa(String.fromCharCode.apply(null, bytes));
    }

    var audio = null;
    function unlock() {
        if (!audio) {
            audio = document.createElement("audio");
            audio.setAttribute("x-webkit-airplay", "deny");
            audio.preload = "auto";
            audio.loop = true;
            audio.src = silentWavUrl();
        }
        var p = audio.play();
        if (p && p.then) p.then(function () { listen(false); }, function () {});
    }
    // Only these events count as a user gesture for play() on touch screens (not pointerdown).
    function listen(on) {
        ["touchend", "pointerup", "click", "keydown"].forEach(function (type) {
            if (on) window.addEventListener(type, unlock, true);
            else window.removeEventListener(type, unlock, true);
        });
    }
    listen(true);
    // Release the element while hidden so it does not hold the session; replay on return.
    document.addEventListener("visibilitychange", function () {
        if (!audio) return;
        if (document.visibilityState === "hidden") {
            audio.pause();
        } else {
            listen(true);
        }
    });
})();
