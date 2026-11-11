function initHls(root) {
  for (const video of root.querySelectorAll('video[data-hls-src]:not([data-hls-ready])')) {
    video.dataset.hlsReady = 'true';
    const link = video.parentElement.querySelector('.video-fallback');
    const fail = () => {
      video.hidden = true;
      if (link) link.hidden = false;
    };

    if (video.canPlayType('application/vnd.apple.mpegurl')) {
      video.src = video.dataset.hlsSrc;
      video.addEventListener('loadedmetadata', () => { if (link) link.hidden = true; }, { once: true });
      video.addEventListener('error', fail, { once: true });
    } else if (window.Hls && Hls.isSupported()) {
      const hls = new Hls({ enableWorker: false });
      hls.loadSource(video.dataset.hlsSrc);
      hls.attachMedia(video);
      hls.on(Hls.Events.MANIFEST_PARSED, () => { if (link) link.hidden = true; });
      hls.on(Hls.Events.ERROR, (_event, data) => {
        if (data.fatal) {
          hls.destroy();
          fail();
        }
      });
    } else {
      fail();
    }
  }
}

document.addEventListener('DOMContentLoaded', () => initHls(document));
document.addEventListener('htmx:afterSwap', (event) => initHls(event.detail.target));
