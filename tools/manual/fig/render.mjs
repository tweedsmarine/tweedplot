import { build } from "esbuild";
import fs from "fs";
const h = `const ATTR={fontSize:'font-size',fontWeight:'font-weight',textAnchor:'text-anchor',strokeWidth:'stroke-width',fillOpacity:'fill-opacity',strokeDasharray:'stroke-dasharray',markerEnd:'marker-end',viewBox:'viewBox',refX:'refX',refY:'refY',markerWidth:'markerWidth',markerHeight:'markerHeight'};
const esc=s=>String(s).replace(/&/g,'&amp;').replace(/</g,'&lt;').replace(/"/g,'&quot;');
globalThis.__h=(t,p,...c)=>{const a=Object.entries(p||{}).map(([k,v])=>' '+(ATTR[k]||k.replace(/[A-Z]/g,m=>'-'+m.toLowerCase()))+'="'+esc(v)+'"').join('');const ch=c.flat(9).map(x=>typeof x==='string'&&!x.startsWith('<')?esc(x):x).join('');return '<'+t+a+'>'+ch+'</'+t+'>';};`;
fs.mkdirSync("out",{recursive:true});
for (const n of ["layout","stages","isochrones"]) {
  const r = await build({ entryPoints:[n+".jsx"], bundle:false, write:false, format:"esm", jsxFactory:"__h", loader:{".jsx":"jsx"} });
  const code = h + r.outputFiles[0].text.replace(/export \{\s*(\w+) as default\s*\};/, "globalThis.W=$1;");
  fs.writeFileSync("out/"+n+".mjs", code);
  await import("./out/"+n+".mjs?"+Date.now());
  let svg = globalThis.W().replace("<svg ", '<svg xmlns="http://www.w3.org/2000/svg" ');
  fs.writeFileSync("out/"+n+".svg", svg);
  console.log(n, svg.length);
}
