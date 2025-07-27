layout(location = 0) in vec3 position;
layout(location = 2) in vec2 uv;

out vec2 vertexUV;

uniform mat4 local_to_world;
uniform mat4 world_to_camera;
uniform mat4 camera_to_clip;

void main() {
    gl_Position = vec4(position, 1) * local_to_world * world_to_camera * camera_to_clip;
    vertexUV = uv;
}