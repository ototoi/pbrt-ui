layout(location = 0) in vec3 position;   //

out vec4 vertexColor;

uniform vec4 base_color;
uniform mat4 local_to_world;
uniform mat4 world_to_camera;
uniform mat4 camera_to_clip;
void main() {
    //gl_Position = camera_to_clip * world_to_camera * local_to_world * vec4(position, 1);
    gl_Position = vec4(position, 1) * local_to_world * world_to_camera * camera_to_clip;
    vertexColor = base_color;
}