/**
 * canvas.js — shared Canvas 2D drawing helpers for the VeloceNetwork Dashboard.
 *
 * All functions are pure (no side-effects beyond drawing onto ctx).
 */

export function clearCanvas(ctx, w, h) {
    ctx.clearRect(0, 0, w, h);
}

/**
 * Draw a filled circle with a centred text label.
 */
export function drawCircle(ctx, x, y, r, fill, label, labelColor = '#fff') {
    ctx.beginPath();
    ctx.arc(x, y, r, 0, Math.PI * 2);
    ctx.fillStyle = fill;
    ctx.fill();
    ctx.strokeStyle = 'rgba(255,255,255,0.15)';
    ctx.lineWidth = 1.5;
    ctx.stroke();

    if (label) {
        ctx.fillStyle = labelColor;
        ctx.font = `bold ${Math.max(10, r * 0.55)}px system-ui, sans-serif`;
        ctx.textAlign = 'center';
        ctx.textBaseline = 'middle';
        // Truncate long names
        const maxChars = Math.floor(r * 0.35);
        const text = label.length > maxChars ? label.slice(0, maxChars) + '…' : label;
        ctx.fillText(text, x, y + r + 14);
    }
}

/**
 * Draw a small rounded rectangle for .vln hostname nodes.
 */
export function drawRect(ctx, x, y, w, h, fill, label, labelColor = '#cdd0d4') {
    const r = 3;
    ctx.beginPath();
    ctx.moveTo(x + r, y);
    ctx.lineTo(x + w - r, y);
    ctx.arcTo(x + w, y, x + w, y + r, r);
    ctx.lineTo(x + w, y + h - r);
    ctx.arcTo(x + w, y + h, x + w - r, y + h, r);
    ctx.lineTo(x + r, y + h);
    ctx.arcTo(x, y + h, x, y + h - r, r);
    ctx.lineTo(x, y + r);
    ctx.arcTo(x, y, x + r, y, r);
    ctx.closePath();
    ctx.fillStyle = fill;
    ctx.fill();
    ctx.strokeStyle = 'rgba(255,255,255,0.1)';
    ctx.lineWidth = 1;
    ctx.stroke();

    if (label) {
        ctx.fillStyle = labelColor;
        ctx.font = '9px system-ui, monospace';
        ctx.textAlign = 'center';
        ctx.textBaseline = 'middle';
        ctx.fillText(label, x + w / 2, y + h / 2);
    }
}

/**
 * Draw a line between two points with variable width and colour.
 */
export function drawEdge(ctx, x1, y1, x2, y2, width, color) {
    ctx.beginPath();
    ctx.moveTo(x1, y1);
    ctx.lineTo(x2, y2);
    ctx.strokeStyle = color;
    ctx.lineWidth = Math.max(1, width);
    ctx.lineCap = 'round';
    ctx.stroke();
}

/**
 * Draw a simple sparkline (line graph) within a bounding box.
 *
 * @param {number[]} points  Array of numeric data points.
 * @param {number} min       Expected minimum value (default 0).
 * @param {number} max       Expected maximum value (default 100).
 */
export function drawSparkline(ctx, x, y, w, h, points, color, min = 0, max = 100) {
    if (!points || points.length < 2) return;
    const range = max - min || 1;
    const step  = w / (points.length - 1);

    ctx.beginPath();
    points.forEach((v, i) => {
        const px = x + i * step;
        const py = y + h - ((v - min) / range) * h;
        if (i === 0) ctx.moveTo(px, py);
        else         ctx.lineTo(px, py);
    });
    ctx.strokeStyle = color;
    ctx.lineWidth   = 1.5;
    ctx.lineJoin    = 'round';
    ctx.stroke();

    // Subtle fill under the line
    ctx.lineTo(x + (points.length - 1) * step, y + h);
    ctx.lineTo(x, y + h);
    ctx.closePath();
    ctx.fillStyle = color.replace(')', ', 0.15)').replace('rgb(', 'rgba(').replace('#', 'rgba(').replace(/rgba\(([0-9a-f]{2})([0-9a-f]{2})([0-9a-f]{2}),/i, (_, r, g, b) =>
        `rgba(${parseInt(r, 16)}, ${parseInt(g, 16)}, ${parseInt(b, 16)},`);
    // Fallback fill
    ctx.fillStyle = 'rgba(99,102,241,0.12)';
    ctx.fill();
}

/**
 * Return a CSS hsl colour interpolating green→yellow→red based on fraction [0, 1].
 * 0.0 = green (hsl 120), 1.0 = red (hsl 0).
 */
export function trafficColor(fraction) {
    const clamped = Math.max(0, Math.min(1, fraction));
    const hue = Math.round(120 * (1 - clamped));
    return `hsl(${hue}, 78%, 46%)`;
}

/**
 * Compute a bytes-per-second rate from two consecutive cumulative snapshots.
 * Returns 0 if timestamps are equal or data is missing.
 *
 * @param {number} bytesNow   Cumulative bytes in latest snapshot.
 * @param {number} bytesPrev  Cumulative bytes in previous snapshot.
 * @param {number} tsNow      Epoch ms of latest snapshot.
 * @param {number} tsPrev     Epoch ms of previous snapshot.
 */
export function bytesPerSec(bytesNow, bytesPrev, tsNow, tsPrev) {
    const dt = (tsNow - tsPrev) / 1000;
    if (dt <= 0) return 0;
    return Math.max(0, (bytesNow - bytesPrev) / dt);
}
