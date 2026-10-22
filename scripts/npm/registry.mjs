import { createServer } from 'node:http';

// No proxy and no persistence: only the supplied, inspected release tarballs exist.
export async function registry(packages) {
  const hits = new Set();
  const byName = new Map(packages.map(p => [p.manifest.name, p]));
  let origin;
  const server = createServer((req, res) => {
    if (req.method !== 'GET') { res.writeHead(405).end(); return; }
    let name;
    try { name = decodeURIComponent(req.url.split('?')[0]).slice(1); } catch { res.writeHead(400).end(); return; }
    if (name.startsWith('tarballs/')) {
      const item = packages.find(p => `tarballs/${p.filename}` === name);
      if (!item) { res.writeHead(404).end(); return; }
      hits.add(`tar:${item.key}`);
      res.writeHead(200, { 'content-type': 'application/octet-stream' }).end(item.bytes);
      return;
    }
    const item = byName.get(name);
    if (!item) { res.writeHead(404, { 'content-type': 'application/json' }).end(JSON.stringify({ error: 'not_found' })); return; }
    hits.add(`metadata:${item.key}`);
    res.writeHead(200, { 'content-type': 'application/json', 'cache-control': 'no-store' });
    res.end(JSON.stringify({ name, 'dist-tags': { latest: item.manifest.version }, versions: { [item.manifest.version]: { ...item.manifest, dist: { tarball: `${origin}/tarballs/${item.filename}`, integrity: item.integrity, shasum: item.shasum } } } }));
  });
  await new Promise((resolve, reject) => { server.once('error', reject); server.listen(0, '127.0.0.1', resolve); });
  origin = `http://127.0.0.1:${server.address().port}`;
  return { origin, hits, close: () => new Promise(resolve => { server.close(resolve); server.closeAllConnections(); }) };
}
