struct Fragment { @location(0) color:vec4<f32>, @location(1) pick:u32 };
@fragment fn fs_main(i:Out)->Fragment{
 var rgb=vec3<f32>(0.34,0.52,0.7)*max(dot(normalize(i.normal),vec3<f32>(0.22,0.72,0.66)),0.12);
 if view.mode.x==1u {rgb=normalize(i.normal)*0.5+0.5;} else if view.mode.x==2u {rgb=vec3<f32>(0.55,0.65,0.8);}
 var out:Fragment;out.color=vec4<f32>(rgb,1.0);out.pick=i.pick;return out;
}
