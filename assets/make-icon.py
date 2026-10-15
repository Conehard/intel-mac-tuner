from PIL import Image, ImageDraw
import numpy as np, math, os
SS=4; S=1024*SS; cx=cy=S//2
def bg(c1=(60,150,250), c2=(18,42,150)):
    margin=int(90*SS); rad=int(205*SS)
    xs=np.linspace(0,1,S); X,Y=np.meshgrid(xs,xs); T=(X+Y)/2.0
    r=(c1[0]+(c2[0]-c1[0])*T).astype('uint8'); g=(c1[1]+(c2[1]-c1[1])*T).astype('uint8'); b=(c1[2]+(c2[2]-c1[2])*T).astype('uint8')
    grad=Image.fromarray(np.dstack([r,g,b,np.full((S,S),255,'uint8')]),"RGBA")
    m=Image.new("L",(S,S),0); ImageDraw.Draw(m).rounded_rectangle((margin,margin,S-margin,S-margin),radius=rad,fill=255)
    im=Image.new("RGBA",(S,S),(0,0,0,0)); im.paste(grad,(0,0),m); return im
def gauge_ticks(span=300,n=13,R1=432,R2=398):
    gl=Image.new("RGBA",(S,S),(0,0,0,0)); gd=ImageDraw.Draw(gl); R1=int(R1*SS); R2=int(R2*SS)
    start=90+(360-span)/2
    for k in range(n):
        a=math.radians(start+span*k/(n-1))
        x1=cx+R1*math.cos(a); y1=cy+R1*math.sin(a); x2=cx+R2*math.cos(a); y2=cy+R2*math.sin(a)
        w=int(24*SS) if k in (0,n-1) else int(14*SS)
        gd.line((x1,y1,x2,y2),fill=(255,255,255,235),width=w)
    return gl
def rocket(scale=0.9,rot=-38):
    rl=Image.new("RGBA",(S,S),(0,0,0,0)); rd=ImageDraw.Draw(rl); s=scale
    bw=int(150*SS*s); bh=int(300*SS*s)
    rd.rounded_rectangle((cx-bw, cy-bh, cx+bw, cy+int(170*SS*s)), radius=int(140*SS*s), fill=(255,255,255,255))
    rd.polygon([(cx-bw, cy-bh+int(60*SS*s)),(cx+bw, cy-bh+int(60*SS*s)),(cx, cy-bh-int(150*SS*s))], fill=(255,255,255,255))
    wy=cy-int(150*SS*s); wr=int(70*SS*s)
    rd.ellipse((cx-wr,wy-wr,cx+wr,wy+wr), fill=(40,120,210,255))
    rd.ellipse((cx-int(38*SS*s),wy-int(38*SS*s),cx+int(38*SS*s),wy+int(38*SS*s)), fill=(20,70,150,255))
    rd.polygon([(cx-bw, cy+int(20*SS*s)),(cx-bw-int(110*SS*s), cy+int(200*SS*s)),(cx-bw, cy+int(160*SS*s))], fill=(255,120,40,255))
    rd.polygon([(cx+bw, cy+int(20*SS*s)),(cx+bw+int(110*SS*s), cy+int(200*SS*s)),(cx+bw, cy+int(160*SS*s))], fill=(255,120,40,255))
    rd.polygon([(cx-int(80*SS*s), cy+int(175*SS*s)),(cx+int(80*SS*s), cy+int(175*SS*s)),(cx, cy+int(360*SS*s))], fill=(255,180,40,255))
    rd.polygon([(cx-int(42*SS*s), cy+int(175*SS*s)),(cx+int(42*SS*s), cy+int(175*SS*s)),(cx, cy+int(290*SS*s))], fill=(255,240,120,255))
    return rl.rotate(rot, center=(cx,cy), resample=Image.BICUBIC)

im=bg(); im=Image.alpha_composite(im, gauge_ticks()); im=Image.alpha_composite(im, rocket(0.9))
master=im.resize((1024,1024), Image.LANCZOS)
os.makedirs("assets/AppIcon.iconset", exist_ok=True)
master.save("assets/icon.png")
# iconset sizes
sizes=[(16,"16x16"),(32,"16x16@2x"),(32,"32x32"),(64,"32x32@2x"),(128,"128x128"),(256,"128x128@2x"),(256,"256x256"),(512,"256x256@2x"),(512,"512x512"),(1024,"512x512@2x")]
for px,name in sizes:
    master.resize((px,px), Image.LANCZOS).save(f"assets/AppIcon.iconset/icon_{name}.png")
master.resize((512,512),Image.LANCZOS).save(f"{os.environ['SCR']}/final-preview.png")
print("wrote assets/icon.png + iconset ("+str(len(sizes))+" sizes)")
