/* ============================================================
   ShaiZero · 官网交互
   轮盘引擎 / 主题预览 / 滚动编排 / 版本信息
   ============================================================ */
(function () {
  "use strict";

  var TAU = Math.PI * 2;
  var SANS = '"PingFang SC","Microsoft YaHei UI","Microsoft YaHei","Hiragino Sans GB","Noto Sans SC",sans-serif';
  var DISPLAY = '"Chakra Petch","PingFang SC",sans-serif';
  var reduce = window.matchMedia && window.matchMedia("(prefers-reduced-motion: reduce)").matches;

  var SEATS = [
    { g: "浏", n: "浏览器", w: "Loop · 设计稿", c: "#4C8DFF" },
    { g: "终", n: "终端", w: "PowerShell 7", c: "#2ED3A0" },
    { g: "编", n: "编辑器", w: "main.rs — shaiwin", c: "#B18CFF" },
    { g: "邮", n: "邮件", w: "收件箱 (12)", c: "#FF7B93" },
    { g: "表", n: "表格", w: "月度成本 · Q4", c: "#F5C451" },
    { g: "图", n: "设计", w: "Aurora Side", c: "#FF8AC2" },
    { g: "播", n: "播放器", w: "Now Playing", c: "#8B93FF" },
    { g: "聊", n: "聊天", w: "Team · 37", c: "#A6E05A" },
    { g: "记", n: "笔记", w: "Daily · 11/11", c: "#8FA3B8" },
    { g: "设", n: "设置", w: "ShaiZero 设置", c: "#45D3F2" }
  ];

  var THEMES = {
    outline: { name: "描边", en: "OUTLINE", desc: "细线、留白，像一张画在桌面上的工程草图。", light: false, gap: 0.012, fill: 0.018, stroke: 0.92, glow: 0, shadow: false },
    glass:   { name: "玻璃", en: "GLASS",   desc: "磨砂玻璃叠在桌面上，默认形态，安静耐看。", light: false, gap: 0.021, fill: 0.2, stroke: 0.72, glow: 18, shadow: false },
    shimmer: { name: "微光", en: "SHIMMER", desc: "边缘带一圈呼吸的微光，夜里会自己亮起来。", light: false, gap: 0.025, fill: 0.16, stroke: 0.68, glow: 32, shadow: false },
    card:    { name: "卡片", en: "CARD",    desc: "厚实色块与清晰分隔，适合浅色桌面。", light: true, gap: 0.030, fill: 0.96, stroke: 0.34, glow: 0, shadow: true },
    neon:    { name: "霓虹", en: "NEON",    desc: "黑场里的灯管，描边发着光，暗处最亮。", light: false, gap: 0.016, fill: 0.085, stroke: 1.0, glow: 42, shadow: false },
    paper:   { name: "纸卡", en: "PAPER",   desc: "奶油纸面与柔和色，白天用不刺眼。", light: true, gap: 0.033, fill: 0.94, stroke: 0.26, glow: 0, shadow: true }
  };

  var themeOrder = ["outline", "glass", "shimmer", "card", "neon", "paper"];

  /* ---------- 颜色 ---------- */
  function rgb(hex) {
    var h = hex.replace("#", "");
    if (h.length === 3) h = h[0] + h[0] + h[1] + h[1] + h[2] + h[2];
    var n = parseInt(h, 16);
    return [n >> 16 & 255, n >> 8 & 255, n & 255];
  }
  function mix(a, b, t) {
    return [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t];
  }
  function rgba(c, a) {
    return "rgba(" + Math.round(c[0]) + "," + Math.round(c[1]) + "," + Math.round(c[2]) + "," + a + ")";
  }
  function clamp(v, a, b) { return v < a ? a : v > b ? b : v; }
  function easeOut(t) { return 1 - Math.pow(1 - t, 3); }
  function fit(ctx, text, maxW) {
    if (ctx.measureText(text).width <= maxW) return text;
    var s = text;
    while (s.length > 1 && ctx.measureText(s + "…").width > maxW) s = s.slice(0, -1);
    return s + "…";
  }

  /* ============================================================
     轮盘
     ============================================================ */
  function Dial(canvas, opts) {
    opts = opts || {};
    this.cv = canvas;
    this.ctx = canvas.getContext("2d");
    this.theme = opts.theme || "glass";
    this.aim = opts.aim !== false;
    this.allowHover = true;
    this.phase = Math.random() * 10;
    this.reveal = 0;
    this.open = 0;
    this.targetOpen = 0;
    this.hover = -1;
    this.flash = -1;
    this.flashT = 0;
    this.pointer = { x: 0, y: 0 };
    this.pointerInside = false;
    this.lift = SEATS.map(function () { return 0; });
    this.visible = true;
    this.w = 0;
    this.h = 0;
    this.dpr = 1;
    this.resize();
    var self = this;
    if (window.ResizeObserver) {
      new ResizeObserver(function () { self.resize(); }).observe(canvas.parentElement);
    } else {
      window.addEventListener("resize", function () { self.resize(); });
    }
  }

  Dial.prototype.resize = function () {
    var p = this.cv.parentElement;
    if (!p) return;
    var rect = p.getBoundingClientRect();
    this.w = Math.max(1, rect.width);
    this.h = Math.max(1, rect.height);
    this.dpr = Math.min(2, window.devicePixelRatio || 1);
    this.cv.width = Math.round(this.w * this.dpr);
    this.cv.height = Math.round(this.h * this.dpr);
    this.cv.style.width = this.w + "px";
    this.cv.style.height = this.h + "px";
  };

  Dial.prototype.geom = function () {
    var size = Math.min(this.w, this.h);
    var R = size * 0.412;
    return { cx: this.w / 2, cy: this.h / 2, R: R, r: R * 0.335 };
  };

  Dial.prototype.hitTest = function (x, y) {
    var g = this.geom();
    var dx = x - g.cx, dy = y - g.cy;
    var d = Math.hypot(dx, dy);
    if (d < g.r * 0.92 || d > g.R * 1.02) return -1;
    var step = TAU / SEATS.length;
    var a = Math.atan2(dy, dx) + Math.PI / 2;
    a = ((a % TAU) + TAU) % TAU;
    return Math.floor(a / step) % SEATS.length;
  };

  Dial.prototype.pick = function (i) {
    this.flash = i;
    this.flashT = 1;
    this.hover = i;
    this.targetOpen = 0;
    var self = this;
    window.setTimeout(function () {
      if (self.flashT <= 0.01 && self.targetOpen < 0.1) self.hover = -1;
    }, 1400);
  };

  Dial.prototype.tick = function (dt) {
    if (!this.visible) return;
    if (!reduce) this.phase += dt;
    this.reveal = Math.min(1, this.reveal + dt * 2.1);
    this.open += (this.targetOpen - this.open) * Math.min(1, dt * 11);
    var h = this.hover;
    for (var i = 0; i < this.lift.length; i++) {
      var goal = h === i ? 1 : 0;
      this.lift[i] += (goal - this.lift[i]) * Math.min(1, dt * 15);
    }
    if (this.flashT > 0) this.flashT = Math.max(0, this.flashT - dt * 1.25);
    this.draw();
  };

  Dial.prototype.draw = function () {
    var ctx = this.ctx;
    var w = this.w, h = this.h;
    if (!w || !h) return;
    ctx.setTransform(this.dpr, 0, 0, this.dpr, 0, 0);
    ctx.clearRect(0, 0, w, h);

    var g = this.geom();
    var cx = g.cx, cy = g.cy;
    var scale = 1 + this.open * 0.012;
    var R = g.R * scale;
    var r = g.r * scale;
    var T = THEMES[this.theme] || THEMES.glass;

    /* 环境光 */
    var amb = ctx.createRadialGradient(cx, cy, R * 0.2, cx, cy, R * 1.4);
    if (T.light) {
      amb.addColorStop(0, "rgba(220,190,150,.16)");
      amb.addColorStop(0.55, "rgba(220,190,150,.04)");
    } else {
      amb.addColorStop(0, "rgba(233,181,106,.13)");
      amb.addColorStop(0.52, "rgba(76,141,255,.055)");
    }
    amb.addColorStop(1, "rgba(0,0,0,0)");
    ctx.fillStyle = amb;
    ctx.fillRect(0, 0, w, h);

    /* 外圈刻度 */
    var tickN = 72;
    for (var i = 0; i < tickN; i++) {
      var a = (i / tickN) * TAU + this.phase * 0.045;
      var major = i % 6 === 0;
      var r0 = R * 1.052;
      var r1 = R * (major ? 1.122 : 1.088);
      ctx.beginPath();
      ctx.moveTo(cx + Math.cos(a) * r0, cy + Math.sin(a) * r0);
      ctx.lineTo(cx + Math.cos(a) * r1, cy + Math.sin(a) * r1);
      ctx.strokeStyle = T.light
        ? "rgba(64,52,38," + (major ? 0.28 : 0.12) + ")"
        : "rgba(242,237,228," + (major ? 0.24 : 0.095) + ")";
      ctx.lineWidth = major ? 1.3 : 1;
      ctx.stroke();
    }

    /* 轨道点 */
    ctx.beginPath();
    ctx.arc(cx, cy, R * 1.18, 0, TAU);
    ctx.setLineDash([1, 8]);
    ctx.strokeStyle = T.light ? "rgba(64,52,38,.16)" : "rgba(242,237,228,.09)";
    ctx.lineWidth = 1;
    ctx.stroke();
    ctx.setLineDash([]);

    var sparks = ["#E9B56A", "#4C8DFF", "#2ED3A0"];
    for (var s = 0; s < sparks.length; s++) {
      var sa = this.phase * (0.12 + s * 0.03) + (s * TAU) / 3;
      var sx = cx + Math.cos(sa) * R * 1.18;
      var sy = cy + Math.sin(sa) * R * 1.18;
      ctx.beginPath();
      ctx.arc(sx, sy, 2.1, 0, TAU);
      ctx.fillStyle = sparks[s];
      ctx.shadowColor = sparks[s];
      ctx.shadowBlur = 12;
      ctx.fill();
      ctx.shadowBlur = 0;
    }

    /* 盘底 */
    ctx.save();
    ctx.shadowColor = "rgba(0,0,0,.55)";
    ctx.shadowBlur = Math.min(70, R * 0.24);
    ctx.shadowOffsetY = R * 0.06;
    var base = ctx.createRadialGradient(cx, cy - R * 0.32, r * 0.4, cx, cy, R * 1.02);
    if (T.light) {
      base.addColorStop(0, "#fbf6ec");
      base.addColorStop(0.58, "#f0e8da");
      base.addColorStop(1, "#ddcfba");
    } else {
      base.addColorStop(0, "#161b23");
      base.addColorStop(0.55, "#0d1117");
      base.addColorStop(1, "#06080b");
    }
    ctx.beginPath();
    ctx.arc(cx, cy, R, 0, TAU);
    ctx.fillStyle = base;
    ctx.fill();
    ctx.restore();

    ctx.beginPath();
    ctx.arc(cx, cy, R, 0, TAU);
    ctx.strokeStyle = T.light ? "rgba(90,72,50,.26)" : "rgba(200,218,240,.18)";
    ctx.lineWidth = 1.2;
    ctx.stroke();

    /* 扇区 */
    var n = SEATS.length;
    var step = TAU / n;
    var start = -Math.PI / 2;
    var gap = T.gap + this.open * 0.007;

    for (var k = 0; k < n; k++) {
      var p = easeOut(clamp((this.reveal - k * 0.026) / 0.7, 0, 1));
      if (p <= 0.002) continue;
      var seat = SEATS[k];
      var col = rgb(seat.c);
      var hot = this.hover === k;
      var lift = this.lift[k];
      var a0 = start + k * step + gap / 2;
      var a1 = start + (k + 1) * step - gap / 2;
      var rr = r + (R - r) * p + lift * Math.max(3, R * 0.028);
      var ri = r - lift * Math.max(1, R * 0.007);

      ctx.save();
      ctx.globalAlpha = p;

      var fillCol, strokeCol;
      if (T.light) {
        var pale = this.theme === "paper" ? [250, 243, 228] : [255, 250, 242];
        var pc = mix(col, pale, (this.theme === "paper" ? 0.82 : 0.60) - lift * 0.08);
        fillCol = rgba(pc, this.theme === "paper" ? 0.94 : 0.98);
        strokeCol = rgba(mix(col, [40, 32, 24], this.theme === "paper" ? 0.42 : 0.32), 0.32 + lift * 0.2);
      } else {
        fillCol = rgba(mix(col, [255, 255, 255], hot ? 0.16 : 0.035), T.fill + lift * 0.15);
        strokeCol = rgba(mix(col, [255, 255, 255], hot ? 0.42 : 0.18), T.stroke + lift * 0.32);
      }

      ctx.beginPath();
      ctx.arc(cx, cy, rr, a0, a1);
      ctx.arc(cx, cy, ri, a1, a0, true);
      ctx.closePath();

      if (T.shadow) {
        ctx.shadowColor = "rgba(20,12,4,.28)";
        ctx.shadowBlur = 16;
        ctx.shadowOffsetY = 6;
      }
      if (T.glow) {
        ctx.shadowColor = rgba(col, hot ? 0.9 : 0.34);
        ctx.shadowBlur = T.glow * (hot ? 1.5 : 0.55);
      }
      ctx.fillStyle = fillCol;
      ctx.fill();
      ctx.shadowBlur = 0;
      ctx.shadowOffsetY = 0;

      ctx.strokeStyle = strokeCol;
      ctx.lineWidth = hot ? 2 : 1.1;
      if (T.glow) {
        ctx.shadowColor = rgba(col, hot ? 0.95 : 0.42);
        ctx.shadowBlur = T.glow * (hot ? 1.6 : 0.7);
      }
      ctx.stroke();
      if (this.theme === "neon") {
        ctx.shadowBlur = 0;
        ctx.strokeStyle = "rgba(255,255,255," + (hot ? 0.65 : 0.34) + ")";
        ctx.lineWidth = 0.8;
        ctx.stroke();
      }
      ctx.shadowBlur = 0;
      ctx.shadowOffsetY = 0;

      /* 玻璃高光 */
      if (!T.light && (this.theme === "glass" || this.theme === "shimmer" || this.theme === "neon")) {
        ctx.beginPath();
        ctx.arc(cx, cy, rr - 1.8, a0 + 0.004, a1 - 0.004);
        ctx.strokeStyle = "rgba(255,255,255," + (hot ? 0.42 : 0.13) + ")";
        ctx.lineWidth = 1;
        ctx.stroke();
      }

      /* 文字 */
      if (p > 0.34) {
        var mid = (a0 + a1) / 2;
        var rad = ri + (rr - ri) * 0.53;
        var tx = cx + Math.cos(mid) * rad;
        var ty = cy + Math.sin(mid) * rad;
        var ink = T.light ? "rgba(28,24,20,.94)" : "rgba(244,239,230,.96)";
        var sub = T.light ? "rgba(48,40,32,.58)" : "rgba(214,226,242,.62)";

        ctx.globalAlpha = p * p;
        ctx.textAlign = "center";
        ctx.textBaseline = "middle";
        ctx.font = "700 " + Math.max(14, R * 0.128).toFixed(1) + "px " + SANS;
        ctx.fillStyle = hot && !T.light ? "#ffffff" : ink;
        ctx.fillText(seat.g, tx, ty - R * 0.044);

        ctx.font = "500 " + Math.max(9, R * 0.047).toFixed(1) + "px " + SANS;
        ctx.fillStyle = hot ? (T.light ? rgba(mix(col, [0, 0, 0], 0.38), 0.9) : rgba(mix(col, [255, 255, 255], 0.55), 0.98)) : sub;
        ctx.fillText(fit(ctx, seat.n, R * 0.34), tx, ty + R * 0.079);
      }

      /* 选中闪光 */
      if (this.flash === k && this.flashT > 0) {
        ctx.globalAlpha = this.flashT;
        ctx.beginPath();
        ctx.arc(cx, cy, rr, a0, a1);
        ctx.arc(cx, cy, ri, a1, a0, true);
        ctx.closePath();
        ctx.strokeStyle = "rgba(255,255,255,.95)";
        ctx.lineWidth = 2 + this.flashT * 3;
        ctx.shadowColor = rgba(col, 0.95);
        ctx.shadowBlur = 28;
        ctx.stroke();
        ctx.shadowBlur = 0;
      }
      ctx.restore();
    }

    /* 微光覆盖 */
    if (this.theme === "shimmer") {
      ctx.save();
      ctx.beginPath();
      ctx.arc(cx, cy, R - 1, 0, TAU);
      ctx.clip();
      var sa = this.phase * 0.22;
      var sh = ctx.createLinearGradient(cx + Math.cos(sa) * R, cy + Math.sin(sa) * R, cx - Math.cos(sa) * R, cy - Math.sin(sa) * R);
      sh.addColorStop(0, "rgba(255,255,255,.015)");
      sh.addColorStop(0.38, "rgba(255,255,255,.025)");
      sh.addColorStop(0.5, "rgba(255,255,255,.13)");
      sh.addColorStop(0.62, "rgba(255,255,255,.025)");
      sh.addColorStop(1, "rgba(255,255,255,.055)");
      ctx.fillStyle = sh;
      ctx.fillRect(cx - R, cy - R, R * 2, R * 2);
      ctx.restore();
    }

    /* 打开时外圈脉冲 */
    if (this.open > 0.02) {
      ctx.beginPath();
      ctx.arc(cx, cy, R * 1.008, 0, TAU);
      ctx.strokeStyle = "rgba(233,181,106," + (0.34 * this.open) + ")";
      ctx.lineWidth = 1.4;
      ctx.stroke();
    }

    /* 中心毂 */
    var hr = r * 0.94;
    ctx.save();
    ctx.shadowColor = "rgba(0,0,0,.55)";
    ctx.shadowBlur = 26;
    ctx.beginPath();
    ctx.arc(cx, cy, hr, 0, TAU);
    ctx.fillStyle = T.light ? "#f9f3e8" : "#080a0e";
    ctx.fill();
    ctx.restore();

    ctx.beginPath();
    ctx.arc(cx, cy, hr, 0, TAU);
    ctx.strokeStyle = T.light ? "rgba(80,64,44,.24)" : "rgba(200,218,240,.24)";
    ctx.lineWidth = 1.1;
    ctx.stroke();

    /* 毂上的强调弧 */
    ctx.beginPath();
    ctx.arc(cx, cy, hr * 0.9, -Math.PI * 0.78, -Math.PI * 0.24);
    ctx.strokeStyle = "rgba(233,181,106,.55)";
    ctx.lineWidth = 1.3;
    ctx.stroke();

    ctx.beginPath();
    ctx.arc(cx, cy, hr * 0.86, 0, TAU);
    ctx.setLineDash([2, 5]);
    ctx.strokeStyle = T.light ? "rgba(80,64,44,.14)" : "rgba(200,218,240,.13)";
    ctx.lineWidth = 1;
    ctx.stroke();
    ctx.setLineDash([]);

    /* 毂上小刻度 */
    for (var q = 0; q < 24; q++) {
      var qa = (q / 24) * TAU + this.phase * 0.045;
      var qr0 = hr * 0.95;
      var qr1 = q % 6 === 0 ? hr * 1.02 : hr * 0.985;
      ctx.beginPath();
      ctx.moveTo(cx + Math.cos(qa) * qr0, cy + Math.sin(qa) * qr0);
      ctx.lineTo(cx + Math.cos(qa) * qr1, cy + Math.sin(qa) * qr1);
      ctx.strokeStyle = T.light ? "rgba(80,64,44,.20)" : "rgba(200,218,240,.18)";
      ctx.lineWidth = 1;
      ctx.stroke();
    }

    var line1, line2, ink1, ink2;
    var compact = this.w < 500;
    if (this.flashT > 0.02 && this.flash >= 0) {
      line1 = "已切换";
      line2 = SEATS[this.flash].n;
    } else if (this.hover >= 0 && this.allowHover) {
      line1 = SEATS[this.hover].n;
      line2 = compact
        ? ("第 " + (this.hover + 1) + " / " + SEATS.length + " 席")
        : (this.open > 0.4 ? "第 " + (this.hover + 1) + " 席 · 松手切换" : "第 " + (this.hover + 1) + " 席 / " + SEATS.length);
    } else {
      line1 = "按住召唤";
      line2 = compact ? "ALT + 左键" : "ALT + 左键 · " + SEATS.length + " 席";
    }
    ink1 = T.light ? "rgba(28,24,20,.95)" : "rgba(244,239,230,.96)";
    ink2 = T.light ? "rgba(48,40,32,.58)" : "rgba(206,220,238,.58)";

    ctx.textAlign = "center";
    ctx.textBaseline = "middle";
    ctx.font = "700 " + Math.max(15, hr * 0.275).toFixed(1) + "px " + SANS;
    ctx.fillStyle = ink1;
    ctx.fillText(fit(ctx, line1, hr * 1.55), cx, cy - hr * 0.16);
    ctx.font = "500 " + Math.max(9, hr * 0.135).toFixed(1) + "px " + SANS;
    ctx.fillStyle = ink2;
    ctx.fillText(fit(ctx, line2, hr * 1.62), cx, cy + hr * 0.24);

    /* 瞄准线 */
    if (this.aim && this.pointerInside && this.hover >= 0) {
      var dx = this.pointer.x - cx;
      var dy = this.pointer.y - cy;
      var dist = Math.hypot(dx, dy);
      if (dist > hr * 1.06) {
        var ang = Math.atan2(dy, dx);
        var end = Math.min(dist, R * 0.985);
        var col2 = rgb(SEATS[this.hover].c);
        var lg = ctx.createLinearGradient(cx, cy, cx + Math.cos(ang) * end, cy + Math.sin(ang) * end);
        lg.addColorStop(0, rgba(col2, 0.05));
        lg.addColorStop(1, rgba(col2, 0.62));
        ctx.beginPath();
        ctx.moveTo(cx + Math.cos(ang) * hr * 1.02, cy + Math.sin(ang) * hr * 1.02);
        ctx.lineTo(cx + Math.cos(ang) * end, cy + Math.sin(ang) * end);
        ctx.strokeStyle = lg;
        ctx.lineWidth = 1;
        ctx.stroke();

        var px = clamp(this.pointer.x, cx - R, cx + R);
        var py = clamp(this.pointer.y, cy - R, cy + R);
        ctx.beginPath();
        ctx.arc(px, py, 4.5, 0, TAU);
        ctx.strokeStyle = rgba(col2, 0.9);
        ctx.lineWidth = 1.4;
        ctx.stroke();
      }
    }

    /* 拾取爆发环 */
    if (this.flashT > 0) {
      var burst = 1 - this.flashT;
      ctx.beginPath();
      ctx.arc(cx, cy, R * (1.01 + burst * 0.075), 0, TAU);
      ctx.strokeStyle = "rgba(233,181,106," + (this.flashT * 0.55) + ")";
      ctx.lineWidth = 1;
      ctx.stroke();
    }
  };

  var DIALS = [];
  function makeDial(canvas, opts) {
    if (!canvas) return null;
    var d = new Dial(canvas, opts);
    DIALS.push(d);
    if ("IntersectionObserver" in window) {
      new IntersectionObserver(function (entries) {
        d.visible = entries[0].isIntersecting;
      }, { threshold: 0 }).observe(canvas.parentElement);
    }
    return d;
  }

  var last = performance.now();
  function frame(ts) {
    var dt = Math.min(0.05, Math.max(0.001, (ts - last) / 1000));
    last = ts;
    for (var i = 0; i < DIALS.length; i++) DIALS[i].tick(dt);
    requestAnimationFrame(frame);
  }

  /* ============================================================
     首屏轮盘
     ============================================================ */
  var heroCv = document.getElementById("heroWheel");
  var hero = makeDial(heroCv, { theme: "glass", aim: true });
  var hudState = document.getElementById("hudState");
  var toast = document.getElementById("toast");
  var toastName = document.getElementById("toastName");
  var toastDot = document.getElementById("toastDot");
  var toastTimer = 0;
  var pressed = false;
  var demoTimers = [];

  function setStatus(cls, text) {
    if (!hudState) return;
    hudState.className = "state" + (cls ? " " + cls : "");
    hudState.textContent = text;
  }

  function showToast(seat) {
    if (!toast || !toastName) return;
    toastName.innerHTML = "已切换至 <em>" + seat.n + "</em>";
    if (toastDot) toastDot.style.background = seat.c;
    toast.classList.remove("show");
    void toast.offsetWidth;
    toast.classList.add("show");
    window.clearTimeout(toastTimer);
    toastTimer = window.setTimeout(function () { toast.classList.remove("show"); }, 2600);
  }

  function clearDemo() {
    for (var i = 0; i < demoTimers.length; i++) window.clearTimeout(demoTimers[i]);
    demoTimers = [];
  }

  function pointOnCanvas(cv, e) {
    var rect = cv.getBoundingClientRect();
    return { x: e.clientX - rect.left, y: e.clientY - rect.top };
  }

  if (hero) {
    heroCv.addEventListener("pointermove", function (e) {
      var p = pointOnCanvas(heroCv, e);
      hero.pointer = p;
      hero.pointerInside = true;
      hero.hover = hero.hitTest(p.x, p.y);
    });
    heroCv.addEventListener("pointerenter", function () {
      hero.pointerInside = true;
    });
    heroCv.addEventListener("pointerleave", function () {
      if (!pressed) {
        hero.pointerInside = false;
        hero.hover = -1;
      }
    });
    heroCv.addEventListener("pointerdown", function (e) {
      if (e.button !== 0 && e.pointerType === "mouse") return;
      e.preventDefault();
      clearDemo();
      pressed = true;
      hero.pointer = pointOnCanvas(heroCv, e);
      hero.pointerInside = true;
      hero.hover = hero.hitTest(hero.pointer.x, hero.pointer.y);
      hero.targetOpen = 1;
      setStatus("open", "OPEN");
      try { heroCv.setPointerCapture(e.pointerId); } catch (err) {}
    });
    window.addEventListener("pointermove", function (e) {
      if (!pressed) return;
      var p = pointOnCanvas(heroCv, e);
      hero.pointer = p;
      hero.hover = hero.hitTest(p.x, p.y);
    });
    window.addEventListener("pointerup", function (e) {
      if (!pressed) return;
      pressed = false;
      var p = pointOnCanvas(heroCv, e);
      hero.pointer = p;
      hero.pointerInside = true;
      var hit = hero.hitTest(p.x, p.y);
      if (hit >= 0) {
        hero.pick(hit);
        showToast(SEATS[hit]);
        setStatus("pick", "PICKED");
        window.setTimeout(function () {
          if (!pressed) setStatus("", "READY");
        }, 2200);
      } else {
        hero.targetOpen = 0;
        setStatus("", "READY");
      }
    });
    window.addEventListener("pointercancel", function () {
      pressed = false;
      hero.targetOpen = 0;
      setStatus("", "READY");
    });

    var demoBtn = document.getElementById("demoBtn");
    if (demoBtn) {
      demoBtn.addEventListener("click", function () {
        clearDemo();
        pressed = false;
        hero.targetOpen = 1;
        hero.pointerInside = true;
        setStatus("open", "OPEN");
        var idx = 4;
        var g = hero.geom();
        var step = TAU / SEATS.length;
        var mid = -Math.PI / 2 + step * (idx + 0.5);
        var rad = (g.r + g.R) * 0.52;
        hero.pointer = { x: g.cx + Math.cos(mid) * rad, y: g.cy + Math.sin(mid) * rad };
        hero.hover = idx;
        demoTimers.push(window.setTimeout(function () {
          hero.hover = idx;
        }, 40));
        demoTimers.push(window.setTimeout(function () {
          hero.targetOpen = 0;
          hero.pick(idx);
          showToast(SEATS[idx]);
          setStatus("pick", "PICKED");
        }, 1050));
        demoTimers.push(window.setTimeout(function () {
          hero.pointerInside = false;
          hero.hover = -1;
          setStatus("", "READY");
        }, 2500));
      });
    }
  }

  /* ============================================================
     形态预览
     ============================================================ */
  var formCv = document.getElementById("formWheel");
  var formDial = makeDial(formCv, { theme: "glass", aim: false });
  var themeName = document.getElementById("themeName");
  var themeEn = document.getElementById("themeEn");
  var themeDesc = document.getElementById("themeDesc");
  var themeNo = document.getElementById("themeNo");
  var themeBtns = Array.prototype.slice.call(document.querySelectorAll(".theme-btn"));

  function applyTheme(key) {
    if (!THEMES[key]) return;
    if (formDial) {
      formDial.theme = key;
      formDial.reveal = 0;
      formDial.hover = -1;
    }
    themeBtns.forEach(function (b) { b.classList.toggle("on", b.getAttribute("data-theme") === key); });
    if (themeName) themeName.textContent = THEMES[key].name;
    if (themeEn) themeEn.textContent = THEMES[key].en;
    if (themeDesc) themeDesc.textContent = THEMES[key].desc;
    if (themeNo) {
      var idx = themeOrder.indexOf(key) + 1;
      themeNo.textContent = (idx < 10 ? "0" : "") + idx + " / 06";
    }
  }

  themeBtns.forEach(function (btn) {
    btn.addEventListener("click", function () { applyTheme(btn.getAttribute("data-theme")); });
  });

  if (formDial) {
    formCv.addEventListener("pointermove", function (e) {
      var p = pointOnCanvas(formCv, e);
      formDial.pointer = p;
      formDial.pointerInside = true;
      formDial.hover = formDial.hitTest(p.x, p.y);
    });
    formCv.addEventListener("pointerleave", function () {
      formDial.pointerInside = false;
      formDial.hover = -1;
    });
  }
  applyTheme("glass");

  /* ============================================================
     滚动编排
     ============================================================ */
  var head = document.querySelector(".site-head");
  var progress = document.querySelector(".progress");
  function onScroll() {
    var y = window.scrollY || window.pageYOffset;
    if (head) head.classList.toggle("scrolled", y > 24);
    if (progress) {
      var max = document.documentElement.scrollHeight - window.innerHeight;
      progress.style.width = (max > 0 ? (y / max) * 100 : 0) + "%";
    }
  }
  window.addEventListener("scroll", onScroll, { passive: true });
  onScroll();

  var revealEls = Array.prototype.slice.call(document.querySelectorAll(".reveal"));
  if ("IntersectionObserver" in window) {
    var rio = new IntersectionObserver(function (entries) {
      entries.forEach(function (en) {
        if (en.isIntersecting) {
          en.target.classList.add("in");
          rio.unobserve(en.target);
        }
      });
    }, { threshold: 0.12, rootMargin: "0px 0px -6% 0px" });
    revealEls.forEach(function (el) { rio.observe(el); });
  } else {
    revealEls.forEach(function (el) { el.classList.add("in"); });
  }

  /* 卡片光斑 */
  Array.prototype.forEach.call(document.querySelectorAll(".card"), function (card) {
    card.addEventListener("pointermove", function (e) {
      var rect = card.getBoundingClientRect();
      card.style.setProperty("--mx", (e.clientX - rect.left) + "px");
      card.style.setProperty("--my", (e.clientY - rect.top) + "px");
    });
  });

  /* ============================================================
     最新版本
     ============================================================ */
  var relVer = document.getElementById("relVer");
  var relVer2 = document.getElementById("relVer2");
  var relSize = document.getElementById("relSize");
  var relDate = document.getElementById("relDate");
  fetch("https://api.github.com/repos/19y/ShaiZero/releases/latest", {
    headers: { Accept: "application/vnd.github+json" }
  })
    .then(function (r) { if (!r.ok) throw new Error("http " + r.status); return r.json(); })
    .then(function (data) {
      if (data && data.tag_name && relVer) {
        relVer.textContent = data.tag_name;
        if (relVer2) relVer2.textContent = data.tag_name;
      }
      if (data && data.assets && data.assets.length) {
        var asset = data.assets.filter(function (a) { return /win64\.zip$/i.test(a.name); })[0] || data.assets[0];
        if (asset && relSize) relSize.textContent = (asset.size / 1048576).toFixed(1) + " MB";
      }
      if (data && data.published_at && relDate) {
        var d = new Date(data.published_at);
        relDate.textContent = d.getFullYear() + "." + String(d.getMonth() + 1).padStart(2, "0") + "." + String(d.getDate()).padStart(2, "0");
      }
    })
    .catch(function () { /* 静态回退值已经在 HTML 里 */ });

  /* 年份 */
  var yearEl = document.getElementById("year");
  if (yearEl) yearEl.textContent = new Date().getFullYear();

  requestAnimationFrame(frame);
})();
