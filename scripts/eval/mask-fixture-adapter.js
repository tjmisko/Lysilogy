// Offline test adapter: execute the exact trusted MuPDF script with synthetic APIs.
const fs = require('node:fs');
const vm = require('node:vm');
const crypto = require('node:crypto');
const packet = JSON.parse(fs.readFileSync(process.argv[2], 'utf8'));
const source = fs.readFileSync(process.argv[3], 'utf8');
const clone = x => JSON.parse(JSON.stringify(x));
const hash = samples => crypto.createHash('sha256').update(Buffer.from(samples)).digest('hex');
const tests = [];
for (const item of packet.cases) {
    const base = item.scene || {scene_events:item.scene_events};
    tests.push({id:item.id,scene:clone(base),expected:item.expected});
    for (const variant of item.isolated_variants || []) {
        const scene=clone(base);
        for (const change of variant.changes) {
            const path=change.path.replace(/\[(\d+)\]/g, '.$1').split('.');
            let at=scene;
            for (const key of path.slice(0,-1)) at=at[key];
            at[path.at(-1)]=change.value_ieee754 === 'NaN' ? NaN : clone(change.value);
        }
        if (variant.rehash_attached_mask) {
            scene.mask.samples_sha256=hash(scene.mask.samples);
            scene.image.attached_mask_sha256=scene.mask.samples_sha256;
        }
        tests.push({id:item.id+'/'+variant.id,scene,expected:variant.expected});
    }
}
function metadata(mask, base=false) {
    const width=mask.width,height=mask.height;
    const components=base?3:1;
    return {
        getWidth:()=>width,getHeight:()=>height,getNumberOfComponents:()=>components,getBitsPerComponent:()=>8,
        getImageMask:()=>false,getInterpolate:()=>mask.interpolate,getColorKey:()=>base?mask.color_key:null,
        getDecode:()=>base?mask.decode_effect:null,getOrientation:()=>0,
        toPixmap:()=>({
            getWidth:()=>width,getHeight:()=>height,getNumberOfComponents:()=>base?3:mask.decoded_components,
            getAlpha:()=>base?mask.decoded_pixmap_alpha:mask.alpha,
            getBounds:()=>[0,0,width,height],getStride:()=>width*components,
            getColorSpace:()=>base?{getNumberOfComponents:()=>3}:null,
            getSample:(x,y)=>mask.samples[y*width+x]
        })
    };
}
const escape = v=>String(v).replace(/&/g,'&amp;').replace(/"/g,'&quot;');
const tag=(kind,object)=>`<${kind} ${Object.entries(object).map(([k,v])=>`${k}="${escape(v)}"`).join(' ')}/>`;
function emit(test) {
    const scenes=test.scene.scene_events || [test.scene];
    const masks=new Map(scenes.map(s=>[s.mask.samples_sha256,s.mask]));
    let body='',printed;
    const page={getBounds:()=>[0,0,...packet.page_dimensions],run:device=>{
        for (const scene of scenes) {
            const mask=scene.mask,image=scene.image;
            const maskTag={transform:mask.matrix.join(' '),width:mask.width,height:mask.height};
            body+=tag('clip_image_mask',maskTag);
            device.clipImageMask(metadata(mask),mask.matrix);
            for (const clip of scene.additional_clips) {
                // The strict trace parser sees this extra scope; Device callbacks
                // for paths are intentionally irrelevant to its image-event list.
                if (clip.kind==='mask') {
                    const extra={...clip,decoded_components:1,alpha:true,interpolate:false};
                    body+=tag('clip_image_mask',{transform:clip.matrix.join(' '),width:clip.width,height:clip.height});
                    device.clipImageMask(metadata(extra),clip.matrix);
                } else {
                    body+='<clip_path winding="nonzero" transform="1 0 0 1 0 0"><moveto x="240" y="120"/><lineto x="280" y="120"/><lineto x="280" y="160"/><lineto x="240" y="160"/><closepath/></clip_path>';
                }
            }
            const attached=masks.get(image.attached_mask_sha256) || {...mask,samples:mask.samples.map(()=>0)};
            const instance=metadata(image,true);
            instance.getMask=()=>scene.event_binding.attached_mask_byte_identity_verified?metadata(attached):null;
            body+=tag('fill_image',{alpha:image.alpha,transform:image.matrix.join(' '),width:image.width,height:image.height});
            device.fillImage(instance,image.matrix,image.alpha);
            for (let i=0;i<=scene.additional_clips.length;++i) body+='<pop_clip/>';
        }
    }};
    vm.runInNewContext(source,{Document:{openDocument:()=>({loadPage:()=>page})},Matrix:{identity:[1,0,0,1,0,0]},scriptArgs:['fixture.pdf','1'],print:s=>{printed=JSON.parse(s);}}, {timeout:1000});
    for (let i=0;i<scenes.length;++i) {
        const binding=scenes[i].event_binding;
        if (!binding.full_device_sequence_verified) printed.operations.pop();
        if (i>0 && binding.image_event===1) printed.operations.at(-1).event_index=1;
    }
    return {id:test.id,expected:test.expected,trace:`<document filename="fixture.pdf"><page number="1" mediabox="0 0 ${packet.page_dimensions.join(' ')}">${body}</page></document>`,receipt:printed};
}
console.log(JSON.stringify({schema_version:1,independent_packet_sha256:crypto.createHash('sha256').update(fs.readFileSync(process.argv[2])).digest('hex'),page_dimensions:packet.page_dimensions,cases:tests.map(emit)}));
