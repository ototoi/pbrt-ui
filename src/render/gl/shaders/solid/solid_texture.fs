precision highp float;
in vec2 vertexUV;
uniform sampler2D base_color;

out vec4 outColor;

void main() {
    outColor = texture(base_color, vertexUV);
}