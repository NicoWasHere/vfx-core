/* Use the Preview host's suites, against the official OpenFX headers. */
#define main preview_main
#include "../../../wrappers/ofx/minihost/minihost.c"
#undef main

static Img history[32], output;
static OfxPropertySetHandle history_props[32], output_props;
static int last_time, reads;
static OfxPropertySetHandle temporal(OfxImageClipHandle c, OfxTime t) {
    if (!strcmp(c->name, "Output")) return output_props;
    CHECK(t >= 0 && t < 32 && t == (int)t, "bad frame time %g", t);
    CHECK(t > last_time, "frames not oldest first: %g after %d", t, last_time);
    CHECK(images_out == 1, "more than one source image held (%d)", images_out);
    last_time = (int)t; reads++;
    return t >= 0 && t < 32 ? history_props[(int)t] : NULL;
}
int main(int argc, char **argv) {
    if (argc != 8) return 2;
    void *lib = open_plugin(argv[1]);
    if (!lib) { open_failed(); return 2; }
    OfxPlugin *(*getp)(int) = plugin_symbol(lib,"OfxGetPlugin");
    plugin = getp(0);
    CHECK(!strcmp(plugin->pluginIdentifier,"com.lofiscifi.SegcamMotionRust"),"wrong identity");
    static OfxHost host; host.host=props_new(); host.fetchSuite=fetchSuite; plugin->setHost(&host);
    CHECK(action(kOfxActionLoad,NULL,NULL,NULL)==kOfxStatOK,"load failed");
    OfxImageEffectHandle e=make_instance(kOfxImageEffectContextFilter);
    int mode=atoi(argv[4]), top=atoi(argv[6]), premult=atoi(argv[7]);
    const char *depth=atoi(argv[5])==8?kOfxBitDepthByte:atoi(argv[5])==16?kOfxBitDepthShort:kOfxBitDepthFloat;
    set_param(e,"output",mode,0);
    FILE *in=fopen(argv[2],"rb"), *out=fopen(argv[3],"wb");
    if (!in || !out) return 2;
    for(int t=0;t<32;t++) {
        history[t]=img_new(0,0,320,180,depth); history[t].top_down=top;
        history[t].premultiplied=premult;
        for(int y=179;y>=0;y--) for(int x=0;x<320;x++) {
            unsigned char argb[4]; if(fread(argb,1,4,in)!=4) return 2;
            float *p=at(&history[t],x,y);
            p[0]=argb[1]/255.f; p[1]=argb[2]/255.f; p[2]=argb[3]/255.f; p[3]=argb[0]/255.f;
            /* Raw fixture already uses the desired alpha convention. */
        }
        int saved=history[t].premultiplied; history[t].premultiplied=0; store(&history[t]); history[t].premultiplied=saved;
        history_props[t]=image_props(&history[t]);
    }
    fclose(in);
    output=img_new(0,0,320,180,depth); output.top_down=top; output.premultiplied=premult; output_props=image_props(&output);
    ClipRec *src=clip_find(e,"Source");
    setDouble(src->props,kOfxImageEffectPropFrameRange,0,0); setDouble(src->props,kOfxImageEffectPropFrameRange,1,31);
    setString(src->props,kOfxImageEffectPropPreMultiplication,0,premult?kOfxImagePreMultiplied:kOfxImageUnPreMultiplied);
    CHECK(find(e->props,kOfxImageEffectPropTemporalClipAccess,0)->i[0]==1,"effect temporal disabled");
    CHECK(find(src->props,kOfxImageEffectPropTemporalClipAccess,0)->i[0]==1,"clip temporal disabled");
    temporal_image=temporal;
    for(int t=0;t<32;t++) {
        OfxPropertySetHandle args=props_new(), needed=props_new();
        setDouble(args,kOfxPropTime,0,t);
        int window[4]={0,0,320,180}; setIntN(args,kOfxImageEffectPropRenderWindow,4,window);
        CHECK(action(kOfxImageEffectActionGetFramesNeeded,e,args,needed)==kOfxStatOK,"frames needed failed");
        Prop *range=find(needed,"OfxImageClipPropFrameRange_Source",0);
        int back=mode==3?39:16;
        CHECK(range && range->dbl[0]==t-back && range->dbl[1]==t,"wrong frame range");
        reads=0; last_time=-1;
        CHECK(action(kOfxImageEffectActionRender,e,args,NULL)==kOfxStatOK,"render failed at %d",t);
        CHECK(images_out==0,"image leak"); CHECK(reads==(t<back?t:back)+1,"wrong read count");
        output.premultiplied=0; load(&output); output.premultiplied=premult;
        for(int y=179;y>=0;y--) for(int x=0;x<320;x++) {
            float *p=at(&output,x,y); unsigned char argb[4]; int order[4]={3,0,1,2};
            for(int c=0;c<4;c++) argb[c]=(unsigned char)(fminf(1,fmaxf(0,p[order[c]]))*255+0.5f);
            fwrite(argb,1,4,out);
        }
    }
    fclose(out); action(kOfxActionUnload,NULL,NULL,NULL);
    printf("mode %d depth %s top %d premult %d: %d failures\n",mode,depth,top,premult,failures);
    return failures?1:0;
}
